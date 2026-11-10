#!/usr/bin/env python3
import importlib.util
import pathlib
import shutil
import tempfile
import unittest
import unittest.mock

SPEC = importlib.util.spec_from_file_location(
    "check_ci_critical_path", pathlib.Path(__file__).with_name("check-ci-critical-path.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CriticalPathTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.workflows = pathlib.Path(self.directory.name)
        for path in MODULE.WORKFLOWS.glob("*.yml"):
            shutil.copy(path, self.workflows / path.name)
        patcher = unittest.mock.patch.object(MODULE, "WORKFLOWS", self.workflows)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.directory.cleanup)

    def edit(self, name, old, new):
        path = self.workflows / name
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1))

    def run_violations(self):
        nodes = MODULE.full_pull_request_run()
        return [
            *MODULE.runner_cap_violations(nodes),
            *MODULE.serial_wait_violations(nodes),
            *MODULE.lto_violations(nodes),
            *MODULE.api_build_violations(nodes),
            *MODULE.cache_violations(),
        ]

    def test_the_current_workflows_pass(self):
        self.assertEqual(self.run_violations(), [])

    def test_more_parallel_jobs_than_the_runner_cap_fail(self):
        self.edit("rust-workspace-checks.yml", "lane: [workspace,", "lane: [extra-one, extra-two, workspace,")
        self.assertEqual(
            self.run_violations(),
            ["a full pull request run can have 21 jobs running at once; GitHub's free plan runs 20"],
        )

    def test_a_heavy_job_waiting_on_another_heavy_job_fails(self):
        self.edit("scope-cli-build.yml", "    needs: prepare\n", "    needs: [prepare, checks]\n")
        self.assertEqual(
            self.run_violations(),
            [
                "scope-cli-build.yml:build (Linux x64) waits for scope-cli-build.yml:checks",
                "scope-cli-build.yml:build (Windows x64) waits for scope-cli-build.yml:checks",
            ],
        )

    def test_pull_request_cli_builds_with_the_shipped_lto_profile_fail(self):
        self.edit("scope-cli-build.yml", '--profile "${{ matrix.profile }}" --locked --target "${{ matrix.target }}" --bin scope\n          fi',
                  '--release --locked --target "${{ matrix.target }}" --bin scope\n          fi')
        violations = self.run_violations()
        self.assertEqual(len(violations), 2)
        self.assertTrue(all("cli/Cargo.toml with profile release (lto=fat, codegen-units=1)" in v for v in violations))

    def test_api_sharing_its_build_job_fails(self):
        self.edit("scope-api-ci.yml", "components: api\n", "components: api git-router\n")
        self.assertEqual(self.run_violations(), ["scope-api-ci.yml:deployment-binaries (api) builds api together with git-router"])

    def test_rust_caches_saved_off_main_fail(self):
        self.edit("scope-integration-ci.yml", "save-if: ${{ github.event_name != 'pull_request' && github.ref == 'refs/heads/main' }}",
                  "save-if: ${{ github.ref != 'refs/heads/feature' }}")
        violations = self.run_violations()
        self.assertEqual(len(violations), 2)
        self.assertIn("saves its cache on pull_request to refs/pull/1/merge", violations[0])
        self.assertIn("saves its cache on push to refs/heads/scope/requests/req_1", violations[1])


class DownloadedKeyTest(unittest.TestCase):
    def test_downloaded_signing_keys_fail_and_pinned_tool_archives_pass(self):
        files = [
            ("key.sh", "curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc -o key.asc\n"),
            ("keyserver.sh", "gpg --recv-keys 0123456789ABCDEF\n"),
            ("tool.sh", "curl -fsSL https://example.com/tool-1.0.tar.gz -o tool.tgz\nsha256sum --check tool.sha256\n"),
        ]
        self.assertEqual(
            MODULE.downloaded_key_violations(files),
            [
                "key.sh downloads a signing key; commit it next to the script that pins it",
                "keyserver.sh downloads a signing key; commit it next to the script that pins it",
            ],
        )


if __name__ == "__main__":
    unittest.main()
