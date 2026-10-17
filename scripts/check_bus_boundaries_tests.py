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


def nested_metadata(root, middle, leaf, middle_source, leaf_source=boundary.REGISTRY):
    packages = [
        {"id": "root", "name": root},
        {"id": "middle", "name": middle, "source": middle_source},
        {"id": "leaf", "name": leaf, "source": leaf_source},
    ]
    nodes = [
        {"id": "root", "deps": [{"pkg": "middle", "dep_kinds": [{"kind": None}]}]},
        {"id": "middle", "deps": [{"pkg": "leaf", "dep_kinds": [{"kind": None}]}]},
        {"id": "leaf", "deps": []},
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

    def test_storage_bus_implementation_and_runtime_closure_is_rejected(self):
        for package in (
            "tinystoragedrivers-client",
            "tinystoragedrivers-core",
            "tinystoragedrivers-sqlite",
            "tinystoragedrivers-mongodb",
            "tokio",
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


if __name__ == "__main__":
    unittest.main()
