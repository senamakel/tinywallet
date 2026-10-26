#!/usr/bin/env python3
"""Audit the bus's normal/build closure using resolved Cargo package identities."""
import json
import subprocess
import sys

ALLOWED = {
    "serde", "serde_core", "serde_derive", "serde_json", "thiserror",
    "thiserror-impl", "proc-macro2", "quote", "syn", "unicode-ident",
    "itoa", "memchr", "zmij",
}
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
TSD_TYPES = "tinystoragedrivers-types"
TSD_TYPES_REV = "c447da31"
TSD_TYPES_SOURCE = f"https://github.com/senamakel/tinystoragedrivers?rev={TSD_TYPES_REV}#"

# Chain, key and crypto implementations never enter the bus closure, not even
# through transitive dependencies.
FORBIDDEN_NAMES = {"bitcoin", "k256"}
FORBIDDEN_PREFIXES = ("coins-",)

# Storage implementation crates must remain behind the host boundary. The
# names are checked against each library's full normal/build closure.
ENGINE_FORBIDDEN = {
    "rusqlite", "libsqlite3-sys", "mongodb", "bson", "aws-sdk-s3", "lapin",
    "amq-protocol", "sqlx", "redb", "sled", "rocksdb",
    "tinystoragedrivers-document", "tinystoragedrivers-kv", "tinystoragedrivers-blob",
    "tinystoragedrivers-queue", "tinystoragedrivers-secrets", "tinystoragedrivers-connect",
    "tinystoragedrivers",
}
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}


def forbidden(name):
    """Whether `name` is a chain/key implementation the bus must never link."""
    normalized = name.replace("_", "-")
    return normalized in FORBIDDEN_NAMES or normalized.startswith(FORBIDDEN_PREFIXES)


def is_canonical_tsd_source(source, *, require_full_hash=False):
    """Whether Cargo resolved TSD types from the pinned fork revision."""
    if not isinstance(source, str):
        return False
    resolved = source.removeprefix("git+")
    if not resolved.startswith(TSD_TYPES_SOURCE):
        return False
    commit = resolved[len(TSD_TYPES_SOURCE):]
    minimum_length = len(TSD_TYPES_REV) if require_full_hash else 8
    return (
        minimum_length <= len(commit) <= len(TSD_TYPES_REV)
        and TSD_TYPES_REV.startswith(commit)
    )


def dependency_allowed(name, source):
    """Whether a bus dependency is registry vocabulary or the pinned TSD DTO crate."""
    if name in ALLOWED and source == REGISTRY:
        return True
    return name == TSD_TYPES and is_canonical_tsd_source(source, require_full_hash=True)


def audit(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in metadata["packages"] if package["name"] == "tinywallet-bus"]
    if len(roots) != 1:
        raise ValueError("expected exactly one tinywallet-bus package")
    queue = [(roots[0], ["tinywallet-bus"])]
    seen = set()
    violations = []
    while queue:
        identity, path = queue.pop()
        if identity in seen:
            continue
        seen.add(identity)
        package = packages[identity]
        if len(path) > 1 and not dependency_allowed(package["name"], package.get("source")):
            violations.append(" -> ".join(path))
        for dependency in nodes[identity]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                child = dependency["pkg"]
                queue.append((child, path + [packages[child]["name"]]))
    return violations


def tree_packages(feature=None):
    """(name, source) of every normal/build dependency of the bus alone.

    `cargo tree -p` resolves features for the selected package only, so a
    workspace sibling's dependencies cannot leak into the bus closure the way
    they do in full-workspace metadata.
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
    """Violations for a `cargo tree` listing."""
    violations = []
    for name, source in packages:
        if forbidden(name):
            violations.append(f"tinywallet-bus -> {name} (forbidden)")
            continue
        if name in ALLOWED and not source:
            continue
        if name == TSD_TYPES and is_canonical_tsd_source(source):
            continue
        violations.append(f"tinywallet-bus -> {name} ({source or 'registry'})")
    return violations


def tree_package_names(output):
    """Extract package names from Cargo's versioned `--prefix none` lines."""
    return [line.split()[0] for line in output.splitlines() if line.split()]


def workspace_library_crates(metadata):
    """Enumerate every library package in this workspace, failing closed."""
    members = metadata.get("workspace_members")
    packages = metadata.get("packages")
    if not isinstance(members, list) or not isinstance(packages, list):
        raise ValueError("Cargo metadata omitted workspace members or packages")
    member_ids = set(members)
    names = sorted({
        package["name"]
        for package in packages
        if package.get("id") in member_ids
        and any(
            LIBRARY_KINDS.intersection(target.get("kind", []))
            for target in package.get("targets", [])
        )
    })
    if not names:
        raise ValueError("Cargo metadata found no workspace library packages")
    return names


def engine_library_crates():
    """Read workspace library package identities without path assumptions."""
    metadata = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return workspace_library_crates(json.loads(metadata))


def engine_tree_packages(crate):
    """Return package names in one library's normal/build dependency tree."""
    command = [
        "cargo", "tree", "-p", crate, "--all-features", "-e", "normal,build",
        "--prefix", "none",
    ]
    out = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    return tree_package_names(out)


def engine_violations(names):
    """Find engine packages by normalized Cargo package name."""
    return sorted({name.replace("_", "-") for name in names} & ENGINE_FORBIDDEN)


if __name__ == "__main__":
    failures = audit_tree(tree_packages())
    if failures:
        print("Forbidden contract dependencies:\n" + "\n".join(failures), file=sys.stderr)
        sys.exit(1)
    for crate in engine_library_crates():
        engines = engine_violations(engine_tree_packages(crate))
        if engines:
            print(f"{crate} normal/build tree contains storage engines: {', '.join(engines)}", file=sys.stderr)
            sys.exit(1)
    print("wallet library normal/build trees contain no storage engine crates")
