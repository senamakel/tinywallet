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
TSD_BUS_TREE_SOURCES = {
    "https://github.com/tinyhumansai/tinystoragedrivers"
    "?rev=c9ea6ff1615b77b55d7269403af5fd11bedc9ce8"
    "#c9ea6ff1615b77b55d7269403af5fd11bedc9ce8",
    "https://github.com/tinyhumansai/tinystoragedrivers"
    "?rev=c9ea6ff1615b77b55d7269403af5fd11bedc9ce8#c9ea6ff1",
}

# The optional `storage` feature exposes only wallet storage names and stored
# record vocabulary. It must not add an implementation/runtime dependency to
# the data-only bus closure.
STORAGE_FEATURE = "storage"

# Chain, key and crypto implementations never enter the bus closure, not even
# through the `storage` feature's registry-crate exemption.
FORBIDDEN_NAMES = {"bitcoin", "k256"}
FORBIDDEN_PREFIXES = ("coins-",)


def forbidden(name):
    """Whether `name` is a chain/key implementation the bus must never link."""
    normalized = name.replace("_", "-")
    return normalized in FORBIDDEN_NAMES or normalized.startswith(FORBIDDEN_PREFIXES)


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


def tree_packages(feature=None):
    """(name, source) of every normal/build dependency of the bus alone.

    `cargo tree -p` resolves features for the selected package only, so a
    workspace sibling that enables `storage` cannot leak into the default run
    the way it does in `cargo metadata`.
    """
    command = ["cargo", "tree", "-p", "tinywallet-bus", "-e", "normal,build", "--prefix", "none", "-f", "{p}"]
    if feature:
        command += ["--features", feature]
    out = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    packages = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 2 or parts[0] == "tinywallet-bus":
            continue
        # Markers such as `(proc-macro)` and `(*)` are not sources; a path or
        # git source is the parenthesised URL or absolute path.
        source = next((part.strip("()") for part in parts[2:] if part.startswith("(") and part not in ("(proc-macro)", "(*)", "(build)")), "")
        packages.append((parts[0], source))
    return packages


def audit_tree(packages):
    """Violations for a `cargo tree` listing of a bus feature closure."""
    violations = []
    for name, source in packages:
        if forbidden(name):
            violations.append(f"tinywallet-bus -> {name} (forbidden)")
            continue
        if name == TSD_BUS and source in TSD_BUS_TREE_SOURCES:
            continue
        if name in ALLOWED and not source:
            continue
        violations.append(f"tinywallet-bus -> {name} ({source or 'registry'})")
    return violations


def _audit_metadata(manifest, package_name, root):
    result = subprocess.run(
        [
            "cargo", "metadata", "--locked", "--format-version", "1",
            "--manifest-path", str(manifest),
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


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    # This isolated consumer resolves its own graph and lockfile, so metadata
    # proves the published data-only contract without workspace feature bleed.
    _audit_metadata(
        root / "proof-workspaces/storage-needs/Cargo.toml",
        "tinywallet-storage-needs-consumer",
        root,
    )
    failures = audit_tree(tree_packages())
    if failures:
        print(
            "Forbidden contract dependencies (default features):\n"
            + "\n".join(failures),
            file=sys.stderr,
        )
        sys.exit(1)
    failures = audit_tree(tree_packages(STORAGE_FEATURE))
    if failures:
        print(
            "Forbidden contract dependencies (storage feature):\n"
            + "\n".join(failures),
            file=sys.stderr,
        )
        sys.exit(1)
    print(
        "tinywallet-bus default and storage-feature closures are data-only; "
        "the excluded consumer stays data-only"
    )
