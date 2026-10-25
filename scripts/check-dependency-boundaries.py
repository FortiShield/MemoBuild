#!/usr/bin/env python3
"""Check direct path dependencies against architecture/dependency-boundaries.json.

This is a lightweight guard, not a complete Rust dependency analyzer.
It deliberately checks only path dependencies between crates listed in policy.
"""
import json
import sys
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[1]
policy_path = root / "architecture" / "dependency-boundaries.json"
if not policy_path.exists():
    print(f"Policy file not found: {policy_path}", file=sys.stderr)
    sys.exit(2)

policy = json.loads(policy_path.read_text())
rules = policy["rules"]
violations = []
checked = 0

for crate, allowed in rules.items():
    manifest_path = root / "crates" / crate / "Cargo.toml"
    if not manifest_path.exists():
        # The scaffold may be partially applied; don't treat absent crates as failures.
        continue
    manifest = tomllib.loads(manifest_path.read_text())
    deps = {}
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        deps.update(manifest.get(section, {}))
    for dep_name, spec in deps.items():
        if not isinstance(spec, dict) or "path" not in spec:
            continue
        target = Path(spec["path"])
        target_name = target.resolve().name
        if target_name not in rules:
            continue
        checked += 1
        if target_name not in allowed:
            violations.append(f"{crate} -> {target_name} is not allowed by policy")

if violations:
    print("Dependency boundary violations:")
    for violation in violations:
        print(f" - {violation}")
    sys.exit(1)

print(f"Dependency boundary check passed ({checked} direct scaffold path dependencies checked).")
