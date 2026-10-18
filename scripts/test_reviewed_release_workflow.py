"""Guard the reviewed-epic release caller and its explicit admission inputs."""

from pathlib import Path
import os
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"
REVIEWED_WORKFLOW = "tinyhumansai/.github/.github/workflows/module-reviewed-epic-release.yml@6ed83b04aab2d8977ec62e966cb914531e534fa4"
LEGACY_WORKFLOW = "tinyhumansai/.github/.github/workflows/module-release.yml@4adc1080fb495098e7af1aade82ed7e6e3c69bb7"
TINYBUS_REV = "cba2de0e573f99fe4c7e45ba14e2d6b5a2a7757f"


class ReviewedReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = WORKFLOW.read_text()

    def test_reviewed_mode_is_opt_in_and_legacy_defaults_to_zero(self):
        self.assertIn(
            "mode:\n        description: Ordinary tag release or reviewed OpenHuman epic release\n"
            "        type: choice",
            self.workflow,
        )
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
            "reviewed-pr: ${{ fromJSON(needs.validate-reviewed-dispatch.outputs.reviewed_pr || '0') }}",
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

    def test_every_workspace_package_is_at_the_feature_release_version(self):
        manifests = [ROOT / "Cargo.toml", *sorted((ROOT / "crates").glob("*/Cargo.toml"))]
        versions = {
            tomllib.loads(manifest.read_text())["package"]["version"]
            for manifest in manifests
        }
        self.assertEqual(versions, {"0.25.0"})

    def validation_run_block(self):
        lines = self.workflow.splitlines()
        start = lines.index("  validate-reviewed-dispatch:") + 1
        end = next(
            index
            for index in range(start, len(lines))
            if lines[index].startswith("  ")
            and not lines[index].startswith("    ")
            and lines[index].endswith(":")
        )
        for index in range(start, end):
            line = lines[index]
            if not line.startswith("        run:"):
                continue
            command = line[len("        run:"):].strip()
            if command != "|":
                return command
            body = []
            for body_line in lines[index + 1:end]:
                if body_line.strip() and not body_line.startswith("          "):
                    break
                body.append(body_line[10:] if body_line.startswith("          ") else "")
            return "\n".join(body)
        raise AssertionError("reviewed-dispatch job has no run block")

    def run_validation_block(self, repository, ref, reviewed_pr):
        with tempfile.TemporaryDirectory(prefix="reviewed-release-test-") as directory:
            empty_cwd = Path(directory) / "empty-cwd"
            empty_cwd.mkdir()
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
                ["bash", "-c", self.validation_run_block()],
                cwd=empty_cwd,
                capture_output=True,
                text=True,
                env=environment,
            )
            return result, output.read_text()

    def test_dispatch_gate_accepts_only_positive_integer_on_canonical_epic_branch(self):
        result, output = self.run_validation_block(
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
                result, output = self.run_validation_block(repository, ref, reviewed_pr)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(output, "")


if __name__ == "__main__":
    unittest.main()
