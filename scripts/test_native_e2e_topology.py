"""Guard native fixture staging, test cardinality, and client coverage scope."""

from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
CLIENT_TESTS = {
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
    def test_client_is_in_the_covered_root_workspace_with_native_targets_gated(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
        self.assertIn("crates/tinywallet-client", workspace["members"])
        self.assertNotIn("crates/tinywallet-client", workspace.get("exclude", []))

        client = tomllib.loads((ROOT / "crates/tinywallet-client/Cargo.toml").read_text())
        self.assertEqual(client["features"]["native-e2e"], [])
        targets = {test["name"]: test for test in client["test"]}
        self.assertEqual(set(targets), set(CLIENT_TESTS))
        for target in targets.values():
            self.assertEqual(target["required-features"], ["native-e2e"])

    def test_all_seven_native_cases_are_required_and_counted_by_the_runner(self):
        self.assertEqual(sum(EXPECTED_COUNTS.values()), 7)
        for target, names in CLIENT_TESTS.items():
            source = (ROOT / "crates/tinywallet-client/tests" / f"{target}.rs").read_text()
            for name in names:
                self.assertRegex(source, rf"(?m)^async fn {re.escape(name)}\(")
            self.assertNotRegex(source, r"(?m)^\s*#\[ignore(?:\(|\])")

        for filename, names in MODULE_TESTS.items():
            source = (ROOT / "crates/tinywallet-module/tests" / filename).read_text()
            for name in names:
                self.assertRegex(source, rf"(?m)^async fn {re.escape(name)}\(")

        shell = (ROOT / "scripts/test-e2e.sh").read_text()
        powershell = (ROOT / "scripts/test-e2e.ps1").read_text()
        for target, expected_count in EXPECTED_COUNTS.items():
            self.assertEqual(shell.count(f"--test {target}"), 1)
            self.assertIn(f"run_counted_test {expected_count} {target}", shell)
            self.assertIn(f"'--test', '{target}'", powershell)
            self.assertIn(f"-Name '{target}' -ExpectedCount {expected_count}", powershell)

        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("module-e2e-script: scripts/test-e2e.sh", ci)

    def test_feature_and_coverage_lanes_stage_the_real_artifact_before_all_features(self):
        feature_shell = (ROOT / "scripts/test-features.sh").read_text()
        self.assertIn("cargo test --locked --workspace", feature_shell)
        self.assertIn("bash scripts/test-e2e.sh --prepare-only", feature_shell)
        self.assertIn("source \"$target/tinywallet-client-e2e.env\"", feature_shell)
        self.assertIn("cargo test --locked --workspace --all-features", feature_shell)

        feature_windows = (ROOT / "scripts/test-features.ps1").read_text()
        self.assertIn(". $nativeSetup -PrepareOnly", feature_windows)
        self.assertIn("cargo test --locked --workspace --all-features", feature_windows)

        coverage = (ROOT / "scripts/check-coverage.sh").read_text()
        self.assertIn("bash scripts/test-e2e.sh --prepare-only", coverage)
        self.assertIn("source \"$target/tinywallet-client-e2e.env\"", coverage)
        self.assertIn("cargo llvm-cov --workspace --all-features --all-targets", coverage)

    def test_windows_archive_operand_is_local_not_a_drive_qualified_remote(self):
        powershell = (ROOT / "scripts/test-e2e.ps1").read_text()
        self.assertIn("Push-Location $target", powershell)
        self.assertIn("tar.exe -czf $archiveName -C 'tinywallet-client-e2e-cache' .", powershell)
        self.assertNotIn("tar.exe -czf $archivePath", powershell)


if __name__ == "__main__":
    unittest.main()
