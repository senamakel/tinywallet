"""Keep artifact-gated wallet tests in their mandatory native runner."""

from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
PROOF = Path("proof-workspaces/tinywallet-client-native-e2e")
PROOF_MANIFEST = ROOT / PROOF / "Cargo.toml"
TARGETS = {
    "native_e2e": (
        "all_six_confidential_wrappers_use_the_verified_unique_peer",
        "wrong_library_pin_prevents_business_delivery_for_all_confidential_methods",
    ),
    "attested_errors": ("invalid_seed_preserves_remote_error_classification_on_strict_delivery",),
    "native_unattested": ("unattested_module_never_receives_any_confidential_wrapper_request",),
}
EXPECTED_COUNTS = {
    "module_e2e": 2,
    "lifecycle_e2e": 1,
    "native_e2e": 2,
    "attested_errors": 1,
    "native_unattested": 1,
}
MODULE_TESTS = {
    "module_e2e.rs": (
        "refuses_a_confidential_call_to_an_unattested_module",
        "the_built_module_signs_every_chain_over_a_real_broker",
    ),
    "lifecycle_e2e.rs": ("the_built_module_signs_every_chain_over_a_real_broker",),
}


class NativeE2ETopologyTests(unittest.TestCase):
    def test_artifact_gated_targets_are_outside_root_workspace_all_features(self):
        root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertIn(str(PROOF), root_manifest["workspace"]["exclude"])
        self.assertIn("crates/tinywallet-client", root_manifest["workspace"]["members"])

        client_manifest = tomllib.loads(
            (ROOT / "crates/tinywallet-client/Cargo.toml").read_text()
        )
        self.assertEqual(client_manifest["features"]["native-e2e"], [])
        self.assertFalse(client_manifest.get("test"))

    def test_native_proof_declares_every_fixture_target_with_an_explicit_feature(self):
        proof_manifest = tomllib.loads(PROOF_MANIFEST.read_text())
        self.assertEqual(proof_manifest["package"]["name"], "tinywallet-client-native-e2e")
        self.assertEqual(set(proof_manifest["features"]["native-e2e"]), {
            "dep:tinywallet-client",
            "tinywallet-client/native-e2e",
        })
        targets = {test["name"]: test for test in proof_manifest["test"]}
        self.assertEqual(set(targets), set(TARGETS))
        for target in targets.values():
            self.assertEqual(target["required-features"], ["native-e2e"])
        proof_lock = tomllib.loads((ROOT / PROOF / "Cargo.lock").read_text())
        tinybus = next(package for package in proof_lock["package"] if package["name"] == "tinybus")
        self.assertEqual(tinybus["version"], "0.3.3")

    def test_native_runners_execute_all_seven_cases_and_ci_invokes_them(self):
        self.assertEqual(sum(EXPECTED_COUNTS.values()), 7)
        source_files = {
            target: (ROOT / PROOF / "tests" / f"{target}.rs").read_text()
            for target in TARGETS
        }
        expected_functions = [name for names in TARGETS.values() for name in names]
        self.assertEqual(len(expected_functions), 4)
        for target, names in TARGETS.items():
            source = source_files[target]
            for name in names:
                self.assertRegex(source, rf"(?m)^async fn {re.escape(name)}\(")
            self.assertNotRegex(source, r"(?m)^\s*#\[ignore(?:\(|\])")

        for filename in ("module_e2e.rs", "lifecycle_e2e.rs"):
            source = (ROOT / "crates/tinywallet-module/tests" / filename).read_text()
            for name in MODULE_TESTS[filename]:
                self.assertRegex(source, rf"(?m)^async fn {re.escape(name)}\(")

        for runner in (ROOT / "scripts/test-e2e.sh", ROOT / "scripts/test-e2e.ps1"):
            source = runner.read_text()
            manifest_path = str(PROOF_MANIFEST.relative_to(ROOT))
            if runner.suffix == ".sh":
                self.assertIn(f"proof_manifest={manifest_path}", source)
            else:
                self.assertIn(f"'--manifest-path', '{manifest_path}'", source)
            for target, expected_count in EXPECTED_COUNTS.items():
                if runner.suffix == ".sh":
                    self.assertEqual(source.count(f"--test {target}"), 1)
                    self.assertIn(f"run_counted_test {expected_count} {target}", source)
                else:
                    self.assertIn(f"'--test', '{target}'", source)
                    self.assertIn(
                        f"-Name '{target}' -ExpectedCount {expected_count}", source
                    )
            self.assertGreaterEqual(source.count("native-e2e"), len(TARGETS))

        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("module-e2e-script: scripts/test-e2e.sh", ci)

    def test_workspace_feature_and_coverage_gates_keep_client_library_in_scope(self):
        feature_runner = (ROOT / "scripts/test-features.sh").read_text()
        self.assertIn("cargo test --locked --workspace", feature_runner)
        self.assertIn("cargo test --locked --workspace --all-features", feature_runner)

        coverage_runner = (ROOT / "scripts/check-coverage.sh").read_text()
        self.assertIn("cargo llvm-cov --workspace --all-features --all-targets", coverage_runner)


if __name__ == "__main__":
    unittest.main()
