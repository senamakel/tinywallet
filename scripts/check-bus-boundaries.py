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


def audit(metadata, root_name="tinywallet-bus", wallet_bus_manifest=None):
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        return ["Cargo metadata is missing its package list"]
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("nodes"), list):
        return ["Cargo metadata is missing resolve nodes"]

    package_ids = [
        package["id"]
        for package in metadata["packages"]
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    ]
    node_ids = [
        node["id"]
        for node in resolve["nodes"]
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    ]
    if len(package_ids) != len(set(package_ids)):
        return ["Cargo metadata contains duplicate package identities"]
    if len(node_ids) != len(set(node_ids)):
        return ["Cargo metadata contains duplicate resolve nodes"]

    packages = {
        package["id"]: package
        for package in metadata["packages"]
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    }
    nodes = {
        node["id"]: node
        for node in resolve["nodes"]
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    }
    roots = [
        package["id"]
        for package in metadata["packages"]
        if isinstance(package, dict)
        and package.get("name") == root_name
        and isinstance(package.get("id"), str)
    ]
    if len(roots) != 1:
        return [f"expected exactly one {root_name} package"]
    if roots[0] not in nodes:
        return [f"Cargo resolve graph is missing root node {root_name}"]
    queue = [(roots[0], [root_name])]
    seen = set()
    violations = []
    while queue:
        identity, path = queue.pop()
        if identity in seen:
            continue
        seen.add(identity)
        package = packages.get(identity)
        if package is None:
            violations.append("unresolved package identity: " + " -> ".join(path))
            continue
        if len(path) > 1:
            name = package["name"]
            source = package.get("source")
            pinned_storage_bus = name == TSD_BUS and source == TSD_BUS_SOURCE
            local_wallet_bus = (
                name == "tinywallet-bus"
                and source is None
                and wallet_bus_manifest is not None
                and package.get("manifest_path") == str(Path(wallet_bus_manifest).resolve())
            )
            registry_contract_dep = name in ALLOWED and source == REGISTRY
            if not (pinned_storage_bus or local_wallet_bus or registry_contract_dep):
                violations.append(" -> ".join(path))
        node = nodes[identity]
        dependencies = node.get("deps")
        if not isinstance(dependencies, list):
            violations.append("resolve node has no dependency list: " + " -> ".join(path))
            continue
        for dependency in dependencies:
            if not isinstance(dependency, dict):
                violations.append("malformed dependency edge: " + " -> ".join(path))
                continue
            child = dependency.get("pkg")
            kinds = dependency.get("dep_kinds")
            if not isinstance(child, str) or not isinstance(kinds, list):
                violations.append("incomplete dependency edge: " + " -> ".join(path))
                continue
            relevant = False
            malformed_kind = False
            for kind in kinds:
                if not isinstance(kind, dict) or "kind" not in kind:
                    malformed_kind = True
                    break
                if kind["kind"] in (None, "build"):
                    relevant = True
                elif kind["kind"] != "dev":
                    malformed_kind = True
                    break
            if malformed_kind:
                violations.append("malformed dependency kind: " + " -> ".join(path))
                continue
            if not relevant:
                continue
            child_package = packages.get(child)
            if child_package is None or child not in nodes:
                name = child_package.get("name", child) if child_package else child
                violations.append(
                    "unresolved dependency node: " + " -> ".join(path + [name])
                )
                continue
            queue.append((child, path + [child_package["name"]]))
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
            failures = audit(
                json.loads(result.stdout),
                package_name,
                root / "crates/tinywallet-bus/Cargo.toml",
            )
            if failures:
                print(
                    f"Forbidden normal/build dependencies from {package_name}:\n"
                    + "\n".join(failures),
                    file=sys.stderr,
                )
                sys.exit(1)
    print("tinywallet-bus and the excluded storage-needs consumer stay data-only")
