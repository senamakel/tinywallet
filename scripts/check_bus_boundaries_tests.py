"""Regression fixtures for implementation edges and registry identity."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).with_name("check-bus-boundaries.py"))
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)


def metadata(name, kind=None, source=boundary.REGISTRY):
    return {"packages": [{"id": "root", "name": "tinywallet-bus"}, {"id": "dependency", "name": name, "source": source}], "resolve": {"nodes": [{"id": "root", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": kind}]}]}, {"id": "dependency", "deps": []}]}}


class ContractBoundaryTests(unittest.TestCase):
    def test_registry_serde_is_allowed(self):
        self.assertEqual(boundary.audit(metadata("serde")), [])

    def test_implementation_is_rejected_even_as_a_build_dependency(self):
        for kind in (None, "build"):
            self.assertEqual(boundary.audit(metadata("tinywallet-crypto", kind)), ["tinywallet-bus -> tinywallet-crypto"])

    def test_dev_dependencies_are_exempt(self):
        self.assertEqual(boundary.audit(metadata("tinywallet-crypto", "dev")), [])

    def test_local_package_cannot_impersonate_serde(self):
        self.assertTrue(boundary.audit(metadata("serde", source=None)))

    def test_pinned_fork_tsd_types_git_source_is_allowed(self):
        self.assertEqual(
            boundary.audit(
                metadata(
                    boundary.TSD_TYPES,
                    source="git+" + boundary.TSD_TYPES_SOURCE
                    + "400b71e5c40fe964130c89e981c64ddeead5619f",
                )
            ),
            [],
        )

    def test_unpinned_fork_tsd_types_source_is_rejected(self):
        self.assertTrue(
            boundary.audit(
                metadata(
                    boundary.TSD_TYPES,
                    source="git+https://github.com/senamakel/tinystoragedrivers"
                    "?tag=v2.1.0#3332061434c01872b0161191268166c12563af87",
                )
            )
        )

    def test_tree_allows_only_the_pinned_fork_source_for_tsd_types(self):
        self.assertEqual(
            boundary.audit_tree([(boundary.TSD_TYPES, boundary.TSD_TYPES_SOURCE + "400b71e5c40fe964130c89e981c64ddeead5619f")]),
            [],
        )
        self.assertTrue(
            boundary.audit_tree(
                [(boundary.TSD_TYPES, boundary.TSD_TYPES_SOURCE + "deadbeef")]
            )
        )
        self.assertTrue(
            boundary.audit_tree(
                [
                    (
                        boundary.TSD_TYPES,
                        "https://github.com/tinyhumansai/tinystoragedrivers"
                        "?tag=v2.1.2#400b71e5c40fe964130c89e981c64ddeead5619f",
                    )
                ]
            )
        )


class EngineTreeTests(unittest.TestCase):
    def test_versioned_cargo_tree_lines_are_checked_by_package_name(self):
        output = """\
tinywallet-module v0.24.1 (/work/tinywallet)
rusqlite v0.40.2 (registry+https://github.com/rust-lang/crates.io-index)
tinystoragedrivers-api v2.1.0 (https://github.com/tinyhumansai/tinystoragedrivers)
"""
        names = boundary.tree_package_names(output)
        self.assertEqual(
            boundary.engine_violations(names),
            ["rusqlite"],
        )

    def test_storage_api_and_types_are_not_engine_crates(self):
        self.assertEqual(
            boundary.engine_violations(
                ["tinystoragedrivers-api", "tinystoragedrivers-types"]
            ),
            [],
        )

    def test_underscore_names_are_normalized_before_engine_matching(self):
        self.assertEqual(boundary.engine_violations(["libsqlite3_sys"]), ["libsqlite3-sys"])

    def test_workspace_library_enumeration_covers_every_library_kind(self):
        kinds = sorted(boundary.LIBRARY_KINDS)
        members = [f"workspace-{index}" for index in range(len(kinds))]
        metadata_value = {
            "workspace_members": members,
            "packages": [
                {
                    "id": member,
                    "name": f"library-{index}",
                    "targets": [{"kind": [kind]}],
                }
                for index, (member, kind) in enumerate(zip(members, kinds, strict=True))
            ]
            + [
                {"id": "binary", "name": "binary-only", "targets": [{"kind": ["bin"]}]},
                {"id": "dependency", "name": "external-library", "targets": [{"kind": ["lib"]}]},
            ],
        }
        self.assertEqual(
            boundary.workspace_library_crates(metadata_value),
            [f"library-{index}" for index in range(len(kinds))],
        )

    def test_workspace_library_enumeration_fails_closed_without_members(self):
        with self.assertRaises(ValueError):
            boundary.workspace_library_crates({"workspace_members": [], "packages": []})

    def test_workspace_library_enumeration_fails_closed_on_missing_metadata(self):
        with self.assertRaises(ValueError):
            boundary.workspace_library_crates({"packages": []})


if __name__ == "__main__":
    unittest.main()
