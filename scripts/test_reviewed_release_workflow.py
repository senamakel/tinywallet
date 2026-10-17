"""Guard the reviewed-epic release caller and its explicit admission inputs."""

from pathlib import Path
import os
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"
VALIDATOR = ROOT / "scripts/validate-reviewed-epic-release.sh"
REVIEWED_WORKFLOW = "tinyhumansai/.github/.github/workflows/module-reviewed-epic-release.yml@b52f22509fdf4969612ac21f225cd0d27c0b3240"
LEGACY_WORKFLOW = "tinyhumansai/.github/.github/workflows/module-release.yml@4adc1080fb495098e7af1aade82ed7e6e3c69bb7"
TINYBUS_REV = "ed1ee4f5ee561c0320fa715eef114e0f1b888e41"


class ReviewedReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = WORKFLOW.read_text()

    def test_reviewed_mode_is_opt_in_and_legacy_defaults_to_zero(self):
        self.assertIn("mode:\n        description: Release path\n        type: choice", self.workflow)
        self.assertIn("default: standard", self.workflow)
        self.assertIn("reviewed_pr:\n        description: Positive canonical PR number", self.workflow)
        self.assertIn("default: '0'", self.workflow)
        self.assertIn("options: [standard, reviewed-epic]", self.workflow)

    def test_ordinary_tag_release_keeps_the_existing_shared_workflow_pin(self):
        self.assertIn(LEGACY_WORKFLOW, self.workflow)
        self.assertIn("module-crate: tinywallet-module", self.workflow)
        self.assertIn("github.event_name == 'push' || inputs.mode == 'standard'", self.workflow)
        self.assertIn("^v(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)", self.workflow)

    def test_reviewed_mode_calls_the_fixed_reusable_workflow_not_its_internal_action(self):
        self.assertIn(REVIEWED_WORKFLOW, self.workflow)
        self.assertNotIn("module-reviewed-epic-preflight@", self.workflow)
        self.assertNotIn("f4602aaa4a6f163973ec38e29f8b5bbc6ff4db51", self.workflow)
        self.assertIn(
            "reviewed-pr: ${{ fromJSON(needs.validate-reviewed-dispatch.outputs.reviewed_pr) }}",
            self.workflow,
        )
        self.assertIn("tag: ${{ needs.resolve.outputs.tag }}", self.workflow)
        self.assertIn("version: ${{ needs.resolve.outputs.version }}", self.workflow)
        self.assertNotIn("module-package:", self.workflow)
        self.assertNotIn("asset-name:", self.workflow)
        self.assertNotIn("extra-packaged-files:", self.workflow)

    def test_permissions_keep_preflight_read_only_and_publish_in_the_shared_workflow(self):
        self.assertIn("permissions: {}", self.workflow)
        self.assertIn(
            "  validate-reviewed-dispatch:\n"
            "    if: github.event_name == 'workflow_dispatch' && inputs.mode == 'reviewed-epic'\n"
            "    needs: [resolve]\n"
            "    permissions: {}",
            self.workflow,
        )
        self.assertIn(
            "  module-reviewed-release:\n"
            "    if: github.event_name == 'workflow_dispatch' && inputs.mode == 'reviewed-epic'\n"
            "    needs: [resolve, validate-reviewed-dispatch]\n"
            "    permissions:\n"
            "      contents: write\n"
            "      pull-requests: read\n"
            "      checks: read\n"
            "      actions: read",
            self.workflow,
        )

    def test_ci_runs_the_behavioral_dispatch_guard(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("python3 scripts/test_reviewed_release_workflow.py", ci)

    def test_module_dependencies_and_nested_patch_use_canonical_tinybus_release(self):
        module_manifest = tomllib.loads((ROOT / "crates/tinywallet-module/Cargo.toml").read_text())
        dependencies = module_manifest["dependencies"]
        dev_dependencies = module_manifest["dev-dependencies"]
        for dep_set, name in ((dependencies, "tinybus"), (dependencies, "tinybus-module"), (dev_dependencies, "tinybus")):
            self.assertEqual(dep_set[name]["rev"], TINYBUS_REV)

        root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        patch_table = root_manifest["patch"]["https://github.com/tinyhumansai/tinybus"]
        self.assertEqual(patch_table["tinybus"]["path"], "vendor/tinybus/crates/tinybus")
        self.assertEqual(patch_table["tinybus-module"]["path"], "vendor/tinybus/crates/tinybus-module")
        commit = subprocess.check_output(
            ["git", "-C", str(ROOT / "vendor/tinybus"), "rev-parse", "HEAD"], text=True
        ).strip()
        self.assertEqual(commit, TINYBUS_REV)

    def run_validator(self, repository, ref, reviewed_pr):
        with tempfile.TemporaryDirectory(prefix="reviewed-release-test-") as directory:
            output = Path(directory) / "output"
            output.write_text("")
            environment = dict(
                os.environ,
                GITHUB_REPOSITORY=repository,
                GITHUB_REF=ref,
                REVIEWED_PR=reviewed_pr,
                GITHUB_OUTPUT=str(output),
            )
            result = subprocess.run(
                ["bash", str(VALIDATOR)], capture_output=True, text=True, env=environment
            )
            return result, output.read_text()

    def test_dispatch_gate_accepts_only_positive_integer_on_canonical_epic_branch(self):
        result, output = self.run_validator(
            "tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "123"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "reviewed_pr=123\n")

        for repository, ref, reviewed_pr in (
            ("senamakel/tinywallet", "refs/heads/epic/8056-modules-wiring", "123"),
            ("tinyhumansai/tinywallet", "refs/heads/main", "123"),
            ("tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "0"),
            ("tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "-1"),
            ("tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "1.5"),
            ("tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "01"),
            ("tinyhumansai/tinywallet", "refs/heads/epic/8056-modules-wiring", "123;echo bad"),
        ):
            with self.subTest(repository=repository, ref=ref, reviewed_pr=reviewed_pr):
                result, output = self.run_validator(repository, ref, reviewed_pr)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(output, "")


if __name__ == "__main__":
    unittest.main()
