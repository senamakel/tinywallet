"""Guard native fixture staging, test cardinality, and client coverage scope."""

from pathlib import Path
import json
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
CLIENT_TESTS = {
    "native_e2e": (
        "all_six_confidential_wrappers_use_the_verified_unique_peer",
        "wrong_library_pin_prevents_business_delivery_for_all_confidential_methods",
        "ordinary_public_call_encodes_transfers_and_keeps_preflight_errors_safe",
    ),
    "attested_errors": ("invalid_seed_preserves_remote_error_classification_on_strict_delivery",),
    "native_unattested": ("unattested_module_never_receives_any_confidential_wrapper_request",),
}
EXPECTED_COUNTS = {
    "module_e2e": 2,
    "lifecycle_e2e": 1,
    "native_e2e": 3,
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


def _job_block(workflow, job_name):
    match = re.search(
        rf"(?ms)^  {re.escape(job_name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
        workflow,
    )
    return match.group(1) if match else ""


def _msrv_contract_issues(workflow):
    issues = []
    matrix_start = workflow.find("msrv-build-targets: |\n")
    matrix_end = workflow.find("      module-e2e-script:", matrix_start)
    if matrix_start < 0 or matrix_end < 0:
        return ["shared MSRV package matrix is missing"]
    targets = json.loads(
        workflow[matrix_start:].split("|\n", 1)[1].split(
            "      module-e2e-script:", 1
        )[0]
    )

    bus_manifest = tomllib.loads((ROOT / "crates/tinywallet-bus/Cargo.toml").read_text())
    module_manifest = tomllib.loads((ROOT / "crates/tinywallet-module/Cargo.toml").read_text())
    bus_floor = bus_manifest["package"]["rust-version"]
    module_floor = module_manifest["package"]["rust-version"]
    if bus_floor != "1.85":
        issues.append("data-only tinywallet-bus floor must remain Rust 1.85")
    if module_floor != "1.88":
        issues.append("tinywallet-module floor must match its Rust 1.88 runtime dependencies")

    bus_wire = any(
        target.get("packages") == ["tinywallet-bus"]
        and target.get("features") == ["wire"]
        and not target.get("all_features", False)
        for target in targets
    )
    if not bus_wire:
        issues.append("shared MSRV matrix must build the data-only bus wire feature at Rust 1.85")
    if any(
        "tinywallet-bus" in target.get("packages", [])
        and target.get("all_features", False)
        for target in targets
    ):
        issues.append("bus all-features must not run at its Rust 1.85 base floor")
    module_matrix = any(
        target.get("packages") == ["tinywallet-module"]
        and target.get("all_targets", False)
        and target.get("all_features", False)
        for target in targets
    )
    if not module_matrix:
        issues.append("shared MSRV matrix must build all module targets/features at its Rust 1.88 floor")

    storage_job = _job_block(workflow, "wallet-storage-msrv")
    if "toolchain: '1.88.0'" not in storage_job:
        issues.append("storage feature job must use Rust 1.88.0")
    if "cargo test --locked -p tinywallet-bus --all-targets --all-features" not in storage_job:
        issues.append("Rust 1.88 storage job must test every bus target with all features")
    if "cargo test --locked -p tinywallet-x402 --features ledger" not in storage_job:
        issues.append("Rust 1.88 storage job must test the x402 ledger consumer")

    proof_job = _job_block(workflow, "wallet-storage-needs-contract")
    if "toolchain: '1.85.1'" not in proof_job or (
        "cargo test --locked --manifest-path proof-workspaces/storage-needs/Cargo.toml"
        not in proof_job
    ):
        issues.append("data-only storage-needs proof must remain tested on Rust 1.85.1")

    bus_docs = (ROOT / "crates/tinywallet-bus/README.md").read_text()
    bus_manifest_text = (ROOT / "crates/tinywallet-bus/Cargo.toml").read_text()
    x402_manifest_text = (ROOT / "crates/tinywallet-x402/Cargo.toml").read_text()
    if "Rust 1.85" not in bus_docs or "Rust 1.88" not in bus_docs or "1.88" not in bus_manifest_text:
        issues.append("bus docs must distinguish the Rust 1.85 contract and Rust 1.88 storage floor")
    if not all(word in x402_manifest_text for word in ("1.88", "ledger", "pay", "tools")):
        issues.append("x402 manifest must document the Rust 1.88 ledger/pay/tools feature floor")
    return issues


class NativeE2ETopologyTests(unittest.TestCase):
    def test_msrv_matrix_splits_the_data_contract_from_storage_runtime_floors(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertEqual(_msrv_contract_issues(workflow), [])

    def test_msrv_contract_rejects_running_bus_storage_at_its_1_85_floor(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        bus_wire = '{"packages":["tinywallet-bus"],"features":["wire"]},'
        self.assertIn(bus_wire, workflow)
        mutated = workflow.replace(
            bus_wire,
            bus_wire
            + '\n          {"packages":["tinywallet-bus"],"all_targets":true,"all_features":true},',
            1,
        )
        self.assertIn(
            "bus all-features must not run at its Rust 1.85 base floor",
            _msrv_contract_issues(mutated),
        )

    def test_msrv_contract_rejects_lowering_the_storage_job_toolchain(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        storage_job = _job_block(workflow, "wallet-storage-msrv")
        self.assertTrue(storage_job)
        mutated_job = storage_job.replace("toolchain: '1.88.0'", "toolchain: '1.85.1'", 1)
        self.assertNotEqual(storage_job, mutated_job)
        mutated = workflow.replace(storage_job, mutated_job, 1)
        self.assertIn(
            "storage feature job must use Rust 1.88.0",
            _msrv_contract_issues(mutated),
        )

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

    def test_all_eight_native_cases_are_required_and_counted_by_the_runner(self):
        self.assertEqual(sum(EXPECTED_COUNTS.values()), 8)
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
        self.assertIn("& cargo test @Arguments", powershell)
        self.assertIn("[string[]]$Arguments", powershell)
        self.assertIn("tar.exe -czf $archiveName -C 'tinywallet-client-e2e-cache' .", powershell)
        self.assertNotIn("tar.exe -czf $archivePath", powershell)

    def test_contributor_checklists_use_fixture_preparing_feature_runner(self):
        for filename in ("AGENTS.md", "CONTRIBUTING.md", "README.md", ".github/PULL_REQUEST_TEMPLATE.md"):
            source = (ROOT / filename).read_text()
            self.assertIn("scripts/test-features.sh", source)
            self.assertNotRegex(source, r"(?m)^cargo test --all-features\s*$")
        self.assertIn("scripts/test-features.ps1", (ROOT / "CONTRIBUTING.md").read_text())
        self.assertIn("scripts/test-features.ps1", (ROOT / ".github/PULL_REQUEST_TEMPLATE.md").read_text())


if __name__ == "__main__":
    unittest.main()
