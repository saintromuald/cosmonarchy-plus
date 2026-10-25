#!/usr/bin/env python3
"""Validate a cosmo_plus.state.json boot report.

Fails (exit 1) when globals_bad > 0, a required feature is missing or
disarmed, or the report is not valid JSON. Prints a compact summary.
"""
import json
import sys


def main(path: str) -> int:
    with open(path, "r", encoding="utf-8") as f:
        state = json.load(f)

    problems = []

    bad = int(state.get("globals_bad", 1))
    if bad != 0:
        bad_ones = [n for n, g in state.get("globals", {}).items() if g.get("ok") == "false"]
        problems.append(f"{bad} global(s) fail plausibility: {bad_ones}")
    pending = [n for n, g in state.get("globals", {}).items() if g.get("ok") == "pending"]
    if pending:
        print(f"  (boot-stage pending: {pending})")

    required = {
        "crash-capture",
        "frame-hook",
        "draw-hook-overlay",
        "idle-workers",
        "tab-cycle",
        "larva-pick",
    }
    for name in sorted(required):
        status = state.get("features", {}).get(name)
        if status is None:
            problems.append(f"feature missing from report: {name}")
        elif status.startswith("disarmed") or status.startswith("disabled"):
            problems.append(f"feature {name}: {status}")

    if not state.get("modules", {}).get("gptp.qdp", {}).get("base"):
        problems.append("gptp.qdp not loaded")

    print(f"== {path} ==")
    print(f"build {state.get('build', {}).get('version')} "
          f"rev {state.get('build', {}).get('rev')}")
    print(f"globals_bad={bad}")
    for name, status in sorted(state.get("features", {}).items()):
        print(f"  {name}: {status}")
    print(f"hooks: {len(state.get('hooks', {}))}, "
          f"config keys: {len(state.get('config_keys', []))}")

    if problems:
        print("FAIL:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <cosmo_plus.state.json>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
