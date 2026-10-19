"""Exercise release gates with real temporary Git repositories and stubbed tools.

No package-registry calls or uploads are made, including in --publish tests.
Run: python3 scripts/test-publish.py
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("publish.sh").resolve()


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="crate-release-test-")
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.remote = root / "origin.git"
        self.bin = root / "bin"
        self.bin.mkdir()
        self.log = root / "calls.jsonl"
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        RELEASE_TEST_LOG=str(self.log), RELEASE_TEST_PUBLISH="yes")
        self.run_git("init", "-q", "--initial-branch=main")
        self.run_git("config", "user.name", "Release Test")
        self.run_git("config", "user.email", "release-test@example.invalid")
        self.run_git("config", "commit.gpgsign", "false")
        self.run_git("config", "tag.gpgsign", "false")
        self.run_git("config", "core.hooksPath", "/dev/null")
        self.run_git("init", "-q", "--bare", "--initial-branch=main", str(self.remote))
        self.run_git("remote", "add", "origin", str(self.remote))
        (self.repo / "scripts").mkdir()
        shutil.copy2(SCRIPT, self.repo / "scripts/publish.sh")
        (self.repo / "Cargo.toml").write_text('[package]\nname = "fixture"\nversion = "0.1.0"\n')
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "Release fixture")
        self.run_git("tag", "-a", "v0.1.0", "-m", "Release")
        self.run_git("push", "-q", "origin", "HEAD", "v0.1.0")
        stub = "#!" + sys.executable + "\n" + '''
import json, os, pathlib, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["RELEASE_TEST_LOG"], "a") as log:
    log.write(json.dumps([tool] + args) + "\\n")
if tool == "cargo" and args[0] == "metadata":
    print(json.dumps({"packages": [{"version": "0.1.0",
        "manifest_path": str(pathlib.Path("Cargo.toml").resolve()),
        "publish": None if os.environ["RELEASE_TEST_PUBLISH"] == "yes" else []}]}))
if tool == "mise":
    if os.environ.get("RELEASE_TEST_DIRTY"):
        pathlib.Path("Cargo.toml").write_text("changed by checks\\n")
    if os.environ.get("RELEASE_TEST_FAIL_CHECK"):
        sys.exit(9)
if tool == "cargo" and "--dry-run" in args and os.environ.get("RELEASE_TEST_FAIL_DRY_RUN"):
    sys.exit(10)
'''
        for name in ["cargo", "mise"]:
            path = self.bin / name
            path.write_text(stub)
            path.chmod(0o755)

    def run_git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env,
                              capture_output=True, text=True, check=True).stdout.strip()

    def release(self, *args):
        return subprocess.run(["bash", "scripts/publish.sh", *args], cwd=self.repo,
                              env=self.env, capture_output=True, text=True)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def assert_blocked(self, message):
        result = self.release("v0.1.0", "--publish")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assertFalse(any(call[:2] == ["cargo", "publish"] for call in self.calls()))

    def test_default_reads_tag_and_only_dry_runs(self):
        result = self.release()
        self.assertEqual(result.returncode, 0, result.stderr)
        publish = [c for c in self.calls() if c[:2] == ["cargo", "publish"]]
        self.assertEqual(publish, [["cargo", "publish", "--dry-run", "--locked", "--registry", "crates-io"]])
        self.assertIn(["mise", "run", "check"], self.calls())

    def test_explicit_publish_runs_dry_run_then_upload(self):
        result = self.release("v0.1.0", "--publish")
        self.assertEqual(result.returncode, 0, result.stderr)
        publish = [c for c in self.calls() if c[:2] == ["cargo", "publish"]]
        self.assertEqual(len(publish), 2)
        self.assertIn("--dry-run", publish[0])
        self.assertEqual(publish[1], ["cargo", "publish", "--locked", "--registry", "crates-io"])

    def test_dirty_tree_blocks(self):
        (self.repo / "untracked.txt").write_text("not committed")
        self.assert_blocked("Working tree must be clean")

    def test_manifest_publication_restriction_blocks(self):
        self.env["RELEASE_TEST_PUBLISH"] = "no"
        self.assert_blocked("does not permit publication")

    def test_wrong_commit_blocks(self):
        self.run_git("commit", "--allow-empty", "-qm", "After release")
        self.assert_blocked("does not point to HEAD")

    def test_version_mismatch_blocks(self):
        self.run_git("tag", "v0.2.0")
        result = self.release("v0.2.0", "--publish")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must match Cargo.toml", result.stderr)

    def test_ambiguous_auto_tag_blocks(self):
        self.run_git("tag", "v0.2.0")
        result = self.release()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one", result.stderr)

    def test_unpushed_tag_blocks(self):
        self.run_git("push", "-q", "origin", ":refs/tags/v0.1.0")
        self.assert_blocked("Push v0.1.0 to origin first")

    def test_changed_remote_tag_blocks(self):
        self.run_git("tag", "-f", "v0.1.0")
        self.assert_blocked("Local and origin release tags differ")

    def test_checks_mutating_files_block_upload(self):
        self.env["RELEASE_TEST_DIRTY"] = "1"
        self.assert_blocked("Release checks changed the working tree")

    def test_failed_checks_block_upload(self):
        self.env["RELEASE_TEST_FAIL_CHECK"] = "1"
        result = self.release("v0.1.0", "--publish")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ["cargo", "publish"] for c in self.calls()))

    def test_failed_dry_run_blocks_upload(self):
        self.env["RELEASE_TEST_FAIL_DRY_RUN"] = "1"
        result = self.release("v0.1.0", "--publish")
        self.assertNotEqual(result.returncode, 0)
        publish = [c for c in self.calls() if c[:2] == ["cargo", "publish"]]
        self.assertEqual(len(publish), 1)
        self.assertIn("--dry-run", publish[0])


if __name__ == "__main__":
    unittest.main()
