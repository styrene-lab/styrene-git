#!/usr/bin/env python3
"""Validate the Heartwood behavioral parity inventory."""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "parity" / "heartwood.toml"
DECISIONS = {"adopt", "defer", "skip"}
STATUSES = {"covered", "partial", "gap", "deferred", "skipped"}
DECISION_STATUSES = {
    "adopt": {"covered", "partial", "gap"},
    "defer": {"deferred"},
    "skip": {"skipped"},
}


def fail(message: str) -> None:
    print(f"heartwood parity: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    with MANIFEST.open("rb") as source:
        manifest = tomllib.load(source)

    if manifest.get("schema") != 1:
        fail("unsupported or missing schema")
    upstream = manifest.get("upstream", {})
    revision = upstream.get("revision", "")
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        fail("upstream revision must be a full lowercase Git object ID")
    if upstream.get("role") != "behavioral-reference":
        fail("upstream role must remain behavioral-reference")

    gates = manifest.get("gates", {})
    if not gates:
        fail("no gates are defined")
    for name, gate in gates.items():
        if gate.get("status") not in {"active", "planned"}:
            fail(f"gate {name!r} has an invalid status")
        if not gate.get("command"):
            fail(f"gate {name!r} has no command")

    checks = manifest.get("checks", [])
    if not checks:
        fail("no parity checks are defined")
    identifiers: set[str] = set()
    seen_decisions: set[str] = set()
    for check in checks:
        identifier = check.get("id", "")
        if not re.fullmatch(r"HW-[A-Z0-9-]+", identifier):
            fail(f"invalid check identifier {identifier!r}")
        if identifier in identifiers:
            fail(f"duplicate check identifier {identifier}")
        identifiers.add(identifier)

        decision = check.get("decision")
        status = check.get("status")
        if decision not in DECISIONS:
            fail(f"{identifier} has invalid decision {decision!r}")
        if status not in STATUSES or status not in DECISION_STATUSES[decision]:
            fail(f"{identifier} status {status!r} does not match decision {decision!r}")
        seen_decisions.add(decision)

        gate_name = check.get("gate")
        if gate_name not in gates:
            fail(f"{identifier} references unknown gate {gate_name!r}")
        if not check.get("upstream"):
            fail(f"{identifier} has no upstream evidence")
        if not check.get("rationale"):
            fail(f"{identifier} has no rationale")

        local = check.get("local", [])
        if status in {"covered", "partial"} and not local:
            fail(f"{identifier} has status {status!r} but no local evidence")
        if status == "covered" and gates[gate_name]["status"] != "active":
            fail(f"{identifier} claims coverage through planned gate {gate_name!r}")
        for relative in local:
            if not (ROOT / relative).exists():
                fail(f"{identifier} local evidence does not exist: {relative}")

    if seen_decisions != DECISIONS:
        fail("inventory must include adopt, defer, and skip decisions")
    print(
        f"heartwood parity: OK ({len(checks)} checks, revision {revision[:12]})"
    )


if __name__ == "__main__":
    main()
