#!/usr/bin/env python3
"""Audit the bus and excluded consumer normal/build closures."""
import json
import subprocess
import sys
from pathlib import Path

ALLOWED = {
    "serde", "serde_core", "serde_derive", "serde_json", "thiserror",
    "thiserror-impl", "proc-macro2", "quote", "syn", "unicode-ident",
    "itoa", "memchr", "zmij",
}
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
TSD_BUS = "tinystoragedrivers-bus"
TSD_BUS_SOURCE = (
    "git+https://github.com/tinyhumansai/tinystoragedrivers"
    "?rev=c9ea6ff1615b77b55d7269403af5fd11bedc9ce8"
    "#c9ea6ff1615b77b55d7269403af5fd11bedc9ce8"
)


def audit(metadata, root_name="tinywallet-bus"):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in metadata["packages"] if package["name"] == root_name]
    if len(roots) != 1:
        raise ValueError(f"expected exactly one {root_name} package")
    queue = [(roots[0], [root_name])]
    seen = set()
    violations = []
    while queue:
        identity, path = queue.pop()
        if identity in seen:
            continue
        seen.add(identity)
        package = packages[identity]
        if len(path) > 1:
            name = package["name"]
            source = package.get("source")
            pinned_storage_bus = name == TSD_BUS and source == TSD_BUS_SOURCE
            local_wallet_bus = name == "tinywallet-bus" and source is None
            registry_contract_dep = name in ALLOWED and source == REGISTRY
            if not (pinned_storage_bus or local_wallet_bus or registry_contract_dep):
                violations.append(" -> ".join(path))
        for dependency in nodes[identity]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                child = dependency["pkg"]
                queue.append((child, path + [packages[child]["name"]]))
    return violations


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    targets = [
        (root / "Cargo.toml", "tinywallet-bus", ([], ["--all-features"])),
        (
            root / "proof-workspaces/storage-needs/Cargo.toml",
            "tinywallet-storage-needs-consumer",
            ([], ["--all-features"]),
        ),
    ]
    for manifest, package_name, feature_sets in targets:
        for features in feature_sets:
            result = subprocess.run(
                [
                    "cargo",
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                    "--manifest-path",
                    str(manifest),
                    *features,
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            failures = audit(json.loads(result.stdout), package_name)
            if failures:
                print(
                    f"Forbidden normal/build dependencies from {package_name}:\n"
                    + "\n".join(failures),
                    file=sys.stderr,
                )
                sys.exit(1)
    print("tinywallet-bus and the excluded storage-needs consumer stay data-only")
