#!/usr/bin/env python3
"""Enforce the layered dependency graph (D-003).

ADE's headline architectural commitment is that no module spans more than one
pipeline layer. Prose cannot enforce that; this can. Every crate's permitted
direct dependencies are declared below, and any deviation fails the build.

The point is not to make cross-layer dependencies impossible — it is to make
adding one a deliberate edit to this file, visible in review, rather than a
line in a Cargo.toml that nobody notices. That is the difference between an
invariant and an intention.

Run: python3 tools/check-layering.py
"""

import json
import subprocess
import sys

# crate -> the direct workspace dependencies it may declare.
#
# Read bottom-up. Layers depend downward on abstractions only: `ade-block`
# defines the BlockSource seam, and `ade-container` / `ade-track` implement it,
# so `ade-block` depends on neither. `ade-core` is the single crate permitted
# to know every layer, because wiring the pipeline is its whole job.
POLICY = {
    "ade-endian":     set(),
    "ade-block":      {"ade-endian"},
    "ade-container":  {"ade-block", "ade-endian"},
    "ade-track":      {"ade-block", "ade-endian"},
    "ade-flux":       {"ade-track", "ade-block"},
    "ade-filesystem": {"ade-block", "ade-endian"},
    "ade-object":     {"ade-filesystem"},
    "ade-catalogue":  {"ade-object"},
    "ade-core": {
        "ade-endian", "ade-block", "ade-container", "ade-track",
        "ade-flux", "ade-filesystem", "ade-object", "ade-catalogue",
    },
    # Front-ends see the core API and nothing else (F-002): no engine logic in
    # UI or CLI code, and no reaching past the seam to a layer directly.
    "ade-cli":        {"ade-core"},
}


def main() -> int:
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            capture_output=True, text=True, check=True,
        ).stdout
    )

    members = {p["name"]: p for p in meta["packages"]}
    failures = []

    undeclared = set(members) - set(POLICY)
    if undeclared:
        failures.append(
            f"crate(s) missing from the layering policy: {', '.join(sorted(undeclared))}\n"
            f"    Add them to tools/check-layering.py, choosing their layer deliberately."
        )

    stale = set(POLICY) - set(members)
    if stale:
        failures.append(f"policy names crate(s) that no longer exist: {', '.join(sorted(stale))}")

    for name, pkg in sorted(members.items()):
        allowed = POLICY.get(name)
        if allowed is None:
            continue
        actual = {
            d["name"] for d in pkg["dependencies"]
            if d["name"] in members and d.get("kind") is None  # normal deps only
        }
        for extra in sorted(actual - allowed):
            failures.append(
                f"{name} depends on {extra}, which its layer does not permit.\n"
                f"    Either the dependency is wrong, or the layering changed — and a\n"
                f"    change to the layering needs a DECISIONS.md entry (D-003)."
            )

    if failures:
        print("Layering check FAILED (D-003):\n", file=sys.stderr)
        for f in failures:
            print(f"  - {f}\n", file=sys.stderr)
        return 1

    print(f"Layering OK — {len(members)} crates, no cross-layer dependencies.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
