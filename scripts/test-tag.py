#!/usr/bin/env python3
"""Tests for scripts/tag.sh using disposable Git repositories.

Cargo is replaced by a stub that reports the fixture manifest's version, so the
tests run without a Rust toolchain. Git and the local bare origin are real.
"""

import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("tag.sh").resolve()

STUB = textwrap.dedent(
    """\
    #!{python}
    import json, re, sys
    from pathlib import Path
    if sys.argv[1:2] == ["metadata"]:
        version = re.search(r'^version = "([^"]+)"', Path("Cargo.toml").read_text(), re.M).group(1)
        print(json.dumps({"packages": [{
            "version": version,
            "manifest_path": str(Path("Cargo.toml").resolve()),
            "publish": None,
        }]}))
    """
)

CARGO_TOML = '[package]\nname = "fixture"\nversion = "{version}"\n'
CHANGELOG = "# Changelog\n\n## 0.1.1 — 2026-01-02\n\n- Add thing.\n\n## 0.1.0 — 2026-01-01\n\n- Initial release.\n"


class TagTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="crate-tag-test-")
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.repo = root / "repo"
        self.remote = root / "origin.git"
        self.bin = root / "bin"
        self.bin.mkdir()
        stub = self.bin / "cargo"
        stub.write_text(STUB.replace("{python}", sys.executable))
        stub.chmod(0o755)
        self.env = {**os.environ, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"}

        self.repo.mkdir()
        self.run_git("init", "-q", "--initial-branch=main")
        self.run_git("config", "user.name", "Test")
        self.run_git("config", "user.email", "test@example.com")
        self.run_git("config", "commit.gpgsign", "false")
        self.run_git("config", "tag.gpgsign", "false")
        self.run_git("config", "core.hooksPath", "/dev/null")
        subprocess.run(["git", "init", "-q", "--bare", "--initial-branch=main", str(self.remote)], check=True)
        self.run_git("remote", "add", "origin", str(self.remote))

        (self.repo / "scripts").mkdir()
        (self.repo / "scripts" / "tag.sh").write_text(SCRIPT.read_text())
        (self.repo / "Cargo.toml").write_text(CARGO_TOML.format(version="0.1.0"))
        (self.repo / "CHANGELOG.md").write_text("# Changelog\n\n## 0.1.0 — 2026-01-01\n\n- Initial release.\n")
        self.run_git("add", "-A")
        self.run_git("commit", "-q", "-m", "Initial release")
        self.run_git("tag", "-a", "v0.1.0", "-m", "Release 0.1.0")
        self.run_git("push", "-q", "origin", "HEAD", "v0.1.0")
        (self.repo / "Cargo.toml").write_text(CARGO_TOML.format(version="0.1.1"))
        (self.repo / "CHANGELOG.md").write_text(CHANGELOG)
        self.run_git("commit", "-q", "-am", "Release 0.1.1")
        self.run_git("push", "-q", "origin", "HEAD")
        self.run_git("remote", "set-head", "origin", "main")

    def commit_changelog(self, text, message):
        (self.repo / "CHANGELOG.md").write_text(text)
        self.run_git("commit", "-q", "-am", message)
        self.run_git("push", "-q", "origin", "HEAD")

    def run_git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, env=self.env, capture_output=True, text=True, check=True
        ).stdout.strip()

    def tag(self, *args):
        return subprocess.run(
            ["bash", "scripts/tag.sh", *args], cwd=self.repo, env=self.env, capture_output=True, text=True
        )

    def remote_refs(self):
        out = subprocess.run(
            ["git", "ls-remote", "--refs", str(self.remote)], capture_output=True, text=True, check=True
        ).stdout
        return {line.split("\t")[1]: line.split("\t")[0] for line in out.splitlines()}

    def local_tags(self):
        return self.run_git("tag", "--list").splitlines()

    def assert_blocked(self, message, *args):
        result = self.tag(*args)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assertEqual(self.local_tags(), ["v0.1.0"])
        self.assertNotIn("refs/tags/v0.1.1", self.remote_refs())

    def assert_annotated(self, tag, version):
        self.assertEqual(self.run_git("cat-file", "-t", tag), "tag")
        self.assertIn(f"Release {version}", self.run_git("tag", "-l", "--format=%(contents)", tag))
        self.assertEqual(self.run_git("rev-parse", f"{tag}^{{commit}}"), self.run_git("rev-parse", "HEAD"))

    def test_no_push_creates_annotated_tag_only(self):
        result = self.tag("--no-push")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_annotated("v0.1.1", "0.1.1")
        self.assertIn("git push origin refs/tags/v0.1.1", result.stdout)
        self.assertNotIn("refs/tags/v0.1.1", self.remote_refs())

    def test_default_pushes_tag(self):
        result = self.tag()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_annotated("v0.1.1", "0.1.1")
        self.assertEqual(self.remote_refs()["refs/tags/v0.1.1"], self.run_git("rev-parse", "v0.1.1"))
        self.assertIn("Pushed v0.1.1", result.stdout)

    def test_unknown_argument_blocks(self):
        self.assert_blocked("Unknown argument: --force", "--force")

    def test_dirty_tree_blocks(self):
        (self.repo / "notes.txt").write_text("wip\n")
        self.assert_blocked("Working tree must be clean")

    def test_feature_branch_blocks(self):
        self.run_git("checkout", "-q", "-b", "feat/thing")
        self.assert_blocked("On feat/thing; releases are tagged on main")

    def test_detached_head_blocks(self):
        self.run_git("checkout", "-q", "--detach")
        self.assert_blocked("Detached HEAD; check out main")

    def test_missing_origin_head_blocks(self):
        self.run_git("remote", "set-head", "origin", "--delete")
        self.assert_blocked("origin has no default branch recorded")

    def test_unpushed_release_commit_blocks(self):
        (self.repo / "src.txt").write_text("more\n")
        self.run_git("add", "-A")
        self.run_git("commit", "-q", "-m", "Unpushed")
        self.assert_blocked("HEAD is not the tip of origin/main")

    def test_divergent_origin_main_blocks(self):
        self.run_git("push", "-q", "--force", "origin", "v0.1.0^{commit}:refs/heads/main")
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("HEAD is not the tip of origin/main", result.stderr)
        self.assertEqual(self.local_tags(), ["v0.1.0"])

    def test_existing_local_tag_blocks(self):
        self.run_git("tag", "v0.1.1", "v0.1.0")
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Tag v0.1.1 already exists locally", result.stderr)
        self.assertNotIn("refs/tags/v0.1.1", self.remote_refs())

    def test_existing_remote_tag_blocks(self):
        self.run_git("tag", "v0.1.1", "v0.1.0")
        self.run_git("push", "-q", "origin", "v0.1.1")
        self.run_git("tag", "-d", "v0.1.1")
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Tag v0.1.1 already exists on origin", result.stderr)
        self.assertEqual(self.local_tags(), ["v0.1.0"])

    def test_unreachable_origin_blocks(self):
        self.run_git("remote", "set-url", "origin", str(self.remote.parent / "missing.git"))
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cannot query origin", result.stderr)
        self.assertEqual(self.local_tags(), ["v0.1.0"])

    def test_missing_changelog_section_blocks(self):
        self.commit_changelog("# Changelog\n\n## 0.1.0 — 2026-01-01\n\n- Initial release.\n", "Forget the changelog")
        self.assert_blocked("must be '## 0.1.1 — YYYY-MM-DD', found '## 0.1.0 — 2026-01-01'")

    def test_unreleased_changelog_section_blocks(self):
        self.commit_changelog(CHANGELOG.replace("0.1.1 — 2026-01-02", "0.1.1 — Unreleased"), "Undated changelog")
        self.assert_blocked("found '## 0.1.1 — Unreleased'")

    def test_buried_changelog_section_blocks(self):
        self.commit_changelog(
            "# Changelog\n\n## 0.2.0 — 2026-01-03\n\n- Later.\n\n## 0.1.1 — 2026-01-02\n\n- Add thing.\n",
            "Newer section on top",
        )
        self.assert_blocked("found '## 0.2.0 — 2026-01-03'")

    def test_rejected_push_keeps_local_tag_only(self):
        hook = self.remote / "hooks" / "pre-receive"
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o755)
        result = self.tag()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Push rejected; v0.1.1 remains local", result.stderr)
        self.assertEqual(self.local_tags(), ["v0.1.0", "v0.1.1"])
        self.assertNotIn("refs/tags/v0.1.1", self.remote_refs())


if __name__ == "__main__":
    unittest.main()
