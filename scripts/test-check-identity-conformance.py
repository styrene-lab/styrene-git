#!/usr/bin/env python3
"""Tests for the exact-revision Identity conformance runner."""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/check-identity-conformance.py"


def command(*arguments: str, cwd: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(arguments, cwd=cwd, text=True, capture_output=True, check=False)


def initialize(checkout: pathlib.Path) -> str:
    command("git", "init", "--quiet", cwd=checkout)
    command("git", "add", ".", cwd=checkout)
    result = command(
        "git",
        "-c",
        "user.name=Conformance Test",
        "-c",
        "user.email=conformance@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "fixture",
        cwd=checkout,
    )
    assert result.returncode == 0, result.stderr
    revision = command("git", "rev-parse", "HEAD", cwd=checkout)
    assert revision.returncode == 0
    return revision.stdout.strip()


def write_identity_fixture(checkout: pathlib.Path) -> None:
    package = checkout / "crates/libs/styrene-identity"
    corpus = package / "tests/vectors/repository-signing-v1"
    corpus.mkdir(parents=True)
    (package / "Cargo.toml").write_text('[package]\nname = "styrene-identity"\nversion = "9.9.9"\n')
    artifacts = []
    for identifier, name, contents in [
        ("repository-signing-positive", "positive.json", b"positive\n"),
        ("repository-signing-negative", "negative.json", b"negative\n"),
    ]:
        path = corpus / name
        path.write_bytes(contents)
        artifacts.append(
            f'[[artifacts]]\nid = "{identifier}"\npath = "crates/libs/styrene-identity/tests/vectors/repository-signing-v1/{name}"\nsha256 = "{hashlib.sha256(contents).hexdigest()}"\n'
        )
    (corpus / "provenance.toml").write_text(
        'schema_version = 1\nprofile = "styrene-repository-signing-v1"\n'
        'status = "candidate"\ngenerator_revision = "PENDING_CLEAN_COMMIT"\n\n'
        + "\n".join(artifacts)
    )


def runner_arguments(
    identity: pathlib.Path,
    git_checkout: pathlib.Path,
    output: pathlib.Path,
    *,
    allow_candidate: bool,
) -> list[str]:
    passing = f'{sys.executable} -c "raise SystemExit(0)"'
    arguments = [
        sys.executable,
        str(SCRIPT),
        "--identity-checkout",
        str(identity),
        "--git-checkout",
        str(git_checkout),
        "--lane",
        "candidate-test",
        "--proptest-seed",
        "0123456789abcdef",
        "--identity-command",
        passing,
        "--git-command",
        passing,
        "--output",
        str(output),
    ]
    if allow_candidate:
        arguments.append("--allow-candidate")
    return arguments


def main() -> int:
    with tempfile.TemporaryDirectory() as temporary:
        root = pathlib.Path(temporary)
        identity = root / "identity"
        git_checkout = root / "git"
        identity.mkdir()
        git_checkout.mkdir()
        write_identity_fixture(identity)
        (git_checkout / "README.md").write_text("fixture\n")
        identity_revision = initialize(identity)
        git_revision = initialize(git_checkout)

        output = root / "report.json"
        success = command(
            *runner_arguments(identity, git_checkout, output, allow_candidate=True), cwd=ROOT
        )
        assert success.returncode == 0, success.stdout + success.stderr
        report = json.loads(output.read_text())
        assert report["success"] is True
        assert report["identity"]["revision"] == identity_revision
        assert report["git"]["revision"] == git_revision
        assert report["identity"]["artifact_sha256"].keys() >= {
            "repository-signing-positive",
            "repository-signing-negative",
        }
        assert report["proptest_seed"] == "0123456789abcdef"
        assert pathlib.Path(report["log"]).is_file()

        provenance = identity / "crates/libs/styrene-identity/tests/vectors/repository-signing-v1/provenance.toml"
        provenance.write_text(
            provenance.read_text()
            .replace('status = "candidate"', 'status = "released"')
            .replace('generator_revision = "PENDING_CLEAN_COMMIT"', f'generator_revision = "{identity_revision}"')
        )
        released_revision = initialize(identity)
        released_output = root / "released.json"
        released = command(
            *runner_arguments(identity, git_checkout, released_output, allow_candidate=False), cwd=ROOT
        )
        assert released.returncode == 0, released.stdout + released.stderr
        released_report = json.loads(released_output.read_text())
        assert released_report["identity"]["revision"] == released_revision
        assert released_report["identity"]["generator_revision"] == identity_revision

        (git_checkout / "dirty").write_text("dirty\n")
        dirty_output = root / "dirty.json"
        failure = command(
            *runner_arguments(identity, git_checkout, dirty_output, allow_candidate=False), cwd=ROOT
        )
        assert failure.returncode == 1
        dirty_report = json.loads(dirty_output.read_text())
        assert dirty_report["success"] is False
        assert "dirty" in dirty_report["error"]

    print("identity conformance harness: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
