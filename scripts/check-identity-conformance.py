#!/usr/bin/env python3
"""Run and record exact-revision Styrene Identity/Git conformance checks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tomllib
from typing import Any


def run(command: list[str], cwd: pathlib.Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


def git(checkout: pathlib.Path, *arguments: str) -> str:
    result = run(["git", *arguments], checkout, os.environ.copy())
    if result.returncode != 0:
        raise ValueError(f"{checkout}: git {' '.join(arguments)} failed: {result.stdout.strip()}")
    return result.stdout.strip()


def checkout_revision(checkout: pathlib.Path) -> str:
    if not checkout.is_dir():
        raise ValueError(f"checkout does not exist: {checkout}")
    root = pathlib.Path(git(checkout, "rev-parse", "--show-toplevel")).resolve()
    if root != checkout.resolve():
        raise ValueError(f"checkout path is not its Git root: {checkout}")
    revision = git(checkout, "rev-parse", "HEAD")
    if len(revision) != 40 or any(character not in "0123456789abcdef" for character in revision):
        raise ValueError(f"checkout did not resolve to a full commit SHA: {checkout}")
    if git(checkout, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ValueError(f"checkout is dirty: {checkout}")
    return revision


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_blob_sha256(checkout: pathlib.Path, revision: str, path: str) -> str:
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=checkout,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"provenance path is absent at {revision}: {path}")
    return hashlib.sha256(result.stdout).hexdigest()


def identity_metadata(checkout: pathlib.Path, revision: str, allow_candidate: bool) -> dict[str, Any]:
    package = tomllib.loads(
        (checkout / "crates/libs/styrene-identity/Cargo.toml").read_text(encoding="utf-8")
    )
    corpus_dir = checkout / "crates/libs/styrene-identity/tests/vectors/repository-signing-v1"
    provenance = tomllib.loads((corpus_dir / "provenance.toml").read_text(encoding="utf-8"))
    status = provenance.get("status")
    generator_revision = provenance.get("generator_revision")
    if status != "released":
        if not allow_candidate or status != "candidate":
            raise ValueError("Identity corpus provenance is not released")
    recorded_revision = isinstance(generator_revision, str) and len(generator_revision) == 40
    if status == "released" and not recorded_revision:
        raise ValueError("released corpus requires a full generator revision")
    if recorded_revision:
        ancestor = run(
            ["git", "merge-base", "--is-ancestor", generator_revision, revision],
            checkout,
            os.environ.copy(),
        )
        if ancestor.returncode != 0:
            raise ValueError("released corpus generator revision is not an ancestor of Identity HEAD")

    artifacts: dict[str, str] = {}
    for section in ["generators", "artifacts"]:
        for entry in provenance.get(section, []):
            path = checkout / entry["path"]
            actual = sha256(path)
            if actual != entry["sha256"]:
                raise ValueError(f"Identity provenance digest mismatch: {entry['path']}")
            if recorded_revision:
                recorded = git_blob_sha256(checkout, generator_revision, entry["path"])
                if recorded != entry["sha256"]:
                    raise ValueError(
                        f"Identity provenance revision digest mismatch: {entry['path']}"
                    )
            if section == "artifacts":
                artifacts[entry["id"]] = actual

    required = {"repository-signing-positive", "repository-signing-negative"}
    if not required.issubset(artifacts):
        raise ValueError("Identity provenance omits a repository-signing corpus artifact")
    return {
        "package_version": package["package"]["version"],
        "profile": provenance["profile"],
        "provenance_status": status,
        "generator_revision": generator_revision,
        "artifact_sha256": artifacts,
    }


def command_argument(value: str) -> list[str]:
    command = shlex.split(value)
    if not command:
        raise argparse.ArgumentTypeError("command cannot be empty")
    return command


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--identity-checkout", required=True, type=pathlib.Path)
    parser.add_argument("--git-checkout", required=True, type=pathlib.Path)
    parser.add_argument("--lane", required=True)
    parser.add_argument("--proptest-seed", required=True)
    parser.add_argument("--identity-command", required=True, type=command_argument)
    parser.add_argument("--git-command", required=True, type=command_argument)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument(
        "--allow-candidate",
        action="store_true",
        help="Permit candidate provenance for harness development; release lanes must not use this",
    )
    args = parser.parse_args()

    report: dict[str, Any] = {
        "schema_version": 1,
        "lane": args.lane,
        "proptest_seed": args.proptest_seed,
        "commands": {
            "identity": args.identity_command,
            "git": args.git_command,
        },
    }
    log_sections: list[str] = []
    exit_code = 1
    try:
        identity_checkout = args.identity_checkout.resolve(strict=True)
        git_checkout = args.git_checkout.resolve(strict=True)
        identity_revision = checkout_revision(identity_checkout)
        git_revision = checkout_revision(git_checkout)
        report["identity"] = {
            "checkout": str(identity_checkout),
            "revision": identity_revision,
            **identity_metadata(identity_checkout, identity_revision, args.allow_candidate),
        }
        report["git"] = {"checkout": str(git_checkout), "revision": git_revision}

        environment = os.environ.copy()
        environment.update(
            {
                "IDENTITY_CHECKOUT": str(identity_checkout),
                "GIT_CHECKOUT": str(git_checkout),
                "PROPTEST_RNG_SEED": args.proptest_seed,
            }
        )
        rustc = run(["rustc", "--version"], git_checkout, environment)
        report["rustc"] = rustc.stdout.strip()

        results: dict[str, int] = {}
        for name, command, checkout in [
            ("identity", args.identity_command, identity_checkout),
            ("git", args.git_command, git_checkout),
        ]:
            result = run(command, checkout, environment)
            results[name] = result.returncode
            log_sections.append(f"$ {shlex.join(command)}\n{result.stdout}")
            if result.returncode != 0:
                break
        report["exit_status"] = results
        exit_code = 0 if results == {"identity": 0, "git": 0} else 1
    except (OSError, KeyError, TypeError, ValueError, tomllib.TOMLDecodeError) as error:
        report["error"] = str(error)
        log_sections.append(str(error))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    log_path = args.output.with_suffix(args.output.suffix + ".log")
    report["log"] = str(log_path)
    report["success"] = exit_code == 0
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    log_path.write_text("\n\n".join(log_sections), encoding="utf-8")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
