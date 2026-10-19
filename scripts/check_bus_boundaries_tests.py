"""Regression fixtures for implementation edges and registry identity."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).with_name("check-bus-boundaries.py"))
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)


def metadata(name, kind=None, source=boundary.REGISTRY):
    return {
        "packages": [
            {"id": "root", "name": "tinywallet-bus"},
            {"id": "dependency", "name": name, "source": source},
        ],
        "resolve": {
            "nodes": [
                {"id": "root", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": kind}]}]},
                {"id": "dependency", "deps": []},
            ]
        },
    }


def nested_metadata(
    root,
    middle,
    leaf,
    middle_source,
    leaf_source=boundary.REGISTRY,
    leaf_features=None,
):
    packages = [
        {"id": "root", "name": root},
        {"id": "middle", "name": middle, "source": middle_source},
        {"id": "leaf", "name": leaf, "source": leaf_source},
    ]
    nodes = [
        {"id": "root", "deps": [{"pkg": "middle", "dep_kinds": [{"kind": None}]}]},
        {"id": "middle", "deps": [{"pkg": "leaf", "dep_kinds": [{"kind": None}]}]},
        {"id": "leaf", "features": leaf_features or [], "deps": []},
    ]
    return {"packages": packages, "resolve": {"nodes": nodes}}


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

    def test_excluded_consumer_must_resolve_the_repository_bus_manifest(self):
        graph = {
            "packages": [
                {"id": "consumer", "name": "tinywallet-storage-needs-consumer"},
                {
                    "id": "wallet-bus",
                    "name": "tinywallet-bus",
                    "source": None,
                    "manifest_path": "/repo/crates/tinywallet-bus/Cargo.toml",
                },
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": "consumer",
                        "deps": [
                            {"pkg": "wallet-bus", "dep_kinds": [{"kind": None}]}
                        ],
                    },
                    {"id": "wallet-bus", "deps": []},
                ]
            },
        }
        self.assertEqual(
            boundary.audit(
                graph,
                "tinywallet-storage-needs-consumer",
                "/repo/crates/tinywallet-bus/Cargo.toml",
            ),
            [],
        )
        self.assertEqual(
            boundary.audit(
                graph,
                "tinywallet-storage-needs-consumer",
                "/another/repo/crates/tinywallet-bus/Cargo.toml",
            ),
            ["tinywallet-storage-needs-consumer -> tinywallet-bus"],
        )

    def test_pinned_storage_bus_data_contract_is_allowed(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "serde",
            boundary.TSD_BUS_SOURCE,
        )
        self.assertEqual(boundary.audit(graph), [])

    def test_storage_bus_client_cannot_pull_tinybus(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "tinybus",
            boundary.TSD_BUS_SOURCE,
            "git+https://github.com/tinyhumansai/tinybus?rev=deadbeef#deadbeef",
        )
        self.assertEqual(
            boundary.audit(graph),
            ["tinywallet-bus -> tinystoragedrivers-bus -> tinybus"],
        )

    def test_storage_bus_must_match_the_locked_release_source(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "serde",
            "git+https://github.com/tinyhumansai/tinystoragedrivers?rev=old#old",
        )
        self.assertEqual(
            boundary.audit(graph),
            ["tinywallet-bus -> tinystoragedrivers-bus"],
        )

    def test_same_sha_from_a_tag_is_not_the_pinned_rev_source(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "serde",
            "git+https://github.com/tinyhumansai/tinystoragedrivers?tag=v1.5.0#c9ea6ff1615b77b55d7269403af5fd11bedc9ce8",
        )
        self.assertEqual(
            boundary.audit(graph),
            ["tinywallet-bus -> tinystoragedrivers-bus"],
        )

    def test_shortened_revision_and_sha_are_not_accepted(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "serde",
            "git+https://github.com/tinyhumansai/tinystoragedrivers?rev=c9ea6ff#c9ea6ff",
        )
        self.assertEqual(
            boundary.audit(graph),
            ["tinywallet-bus -> tinystoragedrivers-bus"],
        )

    def test_a_missing_resolve_node_fails_closed(self):
        graph = nested_metadata(
            "tinywallet-bus",
            boundary.TSD_BUS,
            "serde",
            boundary.TSD_BUS_SOURCE,
        )
        graph["resolve"]["nodes"] = [
            node for node in graph["resolve"]["nodes"] if node["id"] != "leaf"
        ]
        self.assertEqual(
            boundary.audit(graph),
            ["unresolved dependency node: tinywallet-bus -> tinystoragedrivers-bus -> serde"],
        )

    def test_tokio_is_rejected_even_with_extra_default_features(self):
        for features in ([], ["io-std"], ["signal"], ["io-std", "signal"]):
            with self.subTest(features=features):
                graph = nested_metadata(
                    "tinywallet-bus",
                    boundary.TSD_BUS,
                    "tokio",
                    boundary.TSD_BUS_SOURCE,
                    leaf_features=features,
                )
                self.assertEqual(
                    boundary.audit(graph),
                    ["tinywallet-bus -> tinystoragedrivers-bus -> tokio"],
                )

    def test_storage_bus_implementation_and_runtime_closure_is_rejected(self):
        for package in (
            "tinybus",
            "tinybus-client",
            "tinystoragedrivers-client",
            "tinystoragedrivers-core",
            "tinystoragedrivers-busproxy",
            "tinystoragedrivers-secrets",
            "tinystoragedrivers-sqlite",
            "tinystoragedrivers-mongodb",
            "tokio",
            "tinywallet-crypto",
            "tinywallet-x402",
            "tinywallet-web3",
            "tinywallet-module",
        ):
            with self.subTest(package=package):
                graph = nested_metadata(
                    "tinywallet-bus",
                    boundary.TSD_BUS,
                    package,
                    boundary.TSD_BUS_SOURCE,
                    None,
                )
                self.assertEqual(
                    boundary.audit(graph),
                    [f"tinywallet-bus -> tinystoragedrivers-bus -> {package}"],
                )


class StorageContractClosureTests(unittest.TestCase):
    IMPLEMENTATION = ("tinystoragedrivers-core", "https://github.com/tinyhumansai/tinystoragedrivers?tag=v1.0.0#141fd9ed")
    DATA_CONTRACT = (
        boundary.TSD_BUS,
        "https://github.com/tinyhumansai/tinystoragedrivers"
        "?rev=c9ea6ff1615b77b55d7269403af5fd11bedc9ce8#c9ea6ff1",
    )

    def test_pinned_data_only_storage_contract_is_allowed_in_each_feature_closure(self):
        self.assertEqual(boundary.audit_tree([self.DATA_CONTRACT]), [])
        self.assertEqual(boundary.audit_tree([self.DATA_CONTRACT]), [])

    def test_wrong_data_contract_revision_is_rejected_in_both_closures(self):
        wrong = (
            boundary.TSD_BUS,
            "https://github.com/tinyhumansai/tinystoragedrivers"
            "?tag=v1.5.0#c9ea6ff1",
        )
        self.assertEqual(len(boundary.audit_tree([wrong])), 1)

    def test_storage_feature_accepts_only_contract_and_stored_vocabulary_dependencies(self):
        self.assertEqual(
            boundary.audit_tree([
                self.DATA_CONTRACT,
                ("serde", ""),
                ("serde_json", ""),
                ("thiserror", ""),
            ]),
            [],
        )

    def test_storage_feature_rejects_implementation_dependencies(self):
        path = ("tinywallet-crypto", "/work/crates/tinywallet-crypto")
        self.assertEqual(len(boundary.audit_tree([path])), 1)

    def test_storage_feature_rejects_forbidden_registry_crates(self):
        for name in ("coins-bip32", "coins_bip32", "coins-core", "k256", "bitcoin"):
            self.assertEqual(len(boundary.audit_tree([(name, "")]), 1), 1, name)

    def test_storage_feature_rejects_a_foreign_git_source(self):
        fake = ("tinystoragedrivers-bus", "https://example.com/evil")
        self.assertEqual(len(boundary.audit_tree([fake])), 1)

    def test_storage_feature_must_not_link_the_runtime_or_its_native_closure(self):
        for package in (self.PORTS, self.REGISTRY_CRATE, ("tokio", "")):
            name, source = package
            with self.subTest(package=name):
                self.assertEqual(
                    boundary.audit_tree([package]),
                    [f"tinywallet-bus -> {name} ({source or 'registry'})"],
                )


if __name__ == "__main__":
    unittest.main()
