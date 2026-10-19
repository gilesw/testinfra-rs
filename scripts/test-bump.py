#!/usr/bin/env python3
"""Tests for scripts/bump.sh using disposable Git repositories.

Cargo and git-cliff are replaced by stubs that log their arguments, so the
tests run without a Rust toolchain and never touch a registry.
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("bump.sh").resolve()
CLIFF_CONFIG = SCRIPT.parent.parent / "cliff.toml"

STUB = textwrap.dedent(
    """\
    #!{python}
    import json, os, re, sys
    from pathlib import Path
    tool = Path(sys.argv[0]).name
    args = sys.argv[1:]
    with open(os.environ["RELEASE_TEST_LOG"], "a") as log:
        log.write(json.dumps([tool] + args) + "\\n")

    def manifest_version():
        package = Path("Cargo.toml").read_text().split("[dependencies]")[0]
        return re.search(r'^version = "([^"]+)"', package, re.M).group(1)

    if tool == "cargo" and args[:1] == ["metadata"]:
        print(json.dumps({"packages": [{
            "version": manifest_version(),
            "manifest_path": str(Path("Cargo.toml").resolve()),
            "publish": None,
        }]}))
    elif tool == "cargo" and args[:1] == ["update"]:
        if os.environ.get("RELEASE_TEST_FAIL_UPDATE"):
            sys.exit(11)
        lock = Path("Cargo.lock")
        lock.write_text(re.sub(
            r'(name = "fixture"\\nversion = ")[^"]+',
            lambda m: m.group(1) + manifest_version(),
            lock.read_text(),
        ))
    elif tool == "git-cliff":
        if os.environ.get("RELEASE_TEST_FAIL_CLIFF"):
            sys.exit(12)
        version = args[args.index("--tag") + 1].lstrip("v")
        body = os.environ.get("RELEASE_TEST_CLIFF_OUTPUT")
        if body is None:
            body = "## %s — 2026-01-02\\n\\n- Add thing.\\n" % version
        sys.stdout.write(body)
    """
)

CARGO_TOML = textwrap.dedent(
    """\
    [package]
    name = "fixture"
    version = "{version}"
    edition = "2024"

    [dependencies]
    serde = {{ version = "0.1.0" }}
    """
)

CARGO_LOCK = textwrap.dedent(
    """\
    version = 4

    [[package]]
    name = "fixture"
    version = "{version}"
    dependencies = ["serde"]

    [[package]]
    name = "serde"
    version = "0.1.0"
    """
)

CHANGELOG = "# Changelog\n\n## 0.1.0 — 2026-01-01\n\n- Initial release.\n"


class BumpTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="crate-bump-test-")
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.repo = root / "repo"
        self.remote = root / "origin.git"
        self.bin = root / "bin"
        self.log = root / "calls.jsonl"
        self.bin.mkdir()
        for name in ("cargo", "git-cliff"):
            stub = self.bin / name
            stub.write_text(STUB.replace("{python}", sys.executable))
            stub.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "RELEASE_TEST_LOG": str(self.log),
        }

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
        (self.repo / "scripts" / "bump.sh").write_text(SCRIPT.read_text())
        (self.repo / "cliff.toml").write_text(CLIFF_CONFIG.read_text())
        self.write_release_files("0.1.0")
        self.run_git("add", "-A")
        self.run_git("commit", "-q", "-m", "Initial release")
        self.run_git("tag", "-a", "v0.1.0", "-m", "Release 0.1.0")
        self.run_git("push", "-q", "origin", "HEAD", "v0.1.0")
        (self.repo / "src.txt").write_text("thing\n")
        self.run_git("add", "-A")
        self.run_git("commit", "-q", "-m", "Add thing")

    def write_release_files(self, version, changelog=CHANGELOG):
        (self.repo / "Cargo.toml").write_text(CARGO_TOML.format(version=version))
        (self.repo / "Cargo.lock").write_text(CARGO_LOCK.format(version=version))
        (self.repo / "CHANGELOG.md").write_text(changelog)

    def run_git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, env=self.env, capture_output=True, text=True, check=True
        ).stdout.strip()

    def bump(self, *args):
        return subprocess.run(
            ["bash", "scripts/bump.sh", *args], cwd=self.repo, env=self.env, capture_output=True, text=True
        )

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def read(self, name):
        return (self.repo / name).read_text()

    def assert_clean(self):
        self.assertEqual(self.run_git("status", "--porcelain", "--untracked-files=all"), "")

    def assert_blocked(self, message, args=("patch",)):
        head = self.run_git("rev-parse", "HEAD")
        result = self.bump(*args)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assert_clean()
        self.assertEqual(self.run_git("rev-parse", "HEAD"), head)
        self.assertEqual(self.run_git("tag", "--list"), "v0.1.0")

    def assert_bumped(self, level, version):
        result = self.bump(level)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f'version = "{version}"', self.read("Cargo.toml"))
        self.assertIn('serde = { version = "0.1.0" }', self.read("Cargo.toml"))
        self.assertIn(f'name = "fixture"\nversion = "{version}"', self.read("Cargo.lock"))
        self.assertEqual(
            self.read("CHANGELOG.md"),
            f"# Changelog\n\n## {version} — 2026-01-02\n\n- Add thing.\n\n## 0.1.0 — 2026-01-01\n\n- Initial release.\n",
        )
        self.assertEqual(
            sorted(line.split()[-1] for line in self.run_git("status", "--porcelain").splitlines()),
            ["CHANGELOG.md", "Cargo.lock", "Cargo.toml"],
        )
        self.assertEqual(self.run_git("tag", "--list"), "v0.1.0")
        self.assertIn(["cargo", "update", "--workspace"], self.calls())
        self.assertEqual(
            [c for c in self.calls() if c[0] == "git-cliff"],
            [["git-cliff", "--config", "cliff.toml", "--strip", "all", "--tag", f"v{version}", "v0.1.0..HEAD"]],
        )
        self.assertIn(f"Release {version}", result.stdout)

    def test_patch(self):
        self.assert_bumped("patch", "0.1.1")

    def test_minor(self):
        self.assert_bumped("minor", "0.2.0")

    def test_major(self):
        self.assert_bumped("major", "1.0.0")

    def test_missing_level_blocks(self):
        self.assert_blocked("Specify the level", args=())
        self.assertEqual(self.calls(), [])

    def test_unknown_argument_blocks(self):
        self.assert_blocked("Unknown argument: --minor", args=("--minor",))

    def test_two_levels_block(self):
        self.assert_blocked("Specify only one level", args=("minor", "patch"))

    def test_dirty_tree_blocks(self):
        (self.repo / "notes.txt").write_text("wip\n")
        result = self.bump("patch")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Working tree must be clean", result.stderr)
        self.assertEqual(self.calls(), [])
        (self.repo / "notes.txt").unlink()
        self.assert_clean()

    def test_missing_current_tag_blocks(self):
        self.run_git("tag", "-d", "v0.1.0")
        result = self.bump("patch")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Tag v0.1.0 does not exist", result.stderr)
        self.assert_clean()

    def test_newer_reachable_tag_blocks(self):
        self.run_git("tag", "v0.2.0")
        result = self.bump("patch")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Newest release tag reachable from HEAD is v0.2.0", result.stderr)
        self.assert_clean()

    def test_no_commits_since_tag_blocks(self):
        self.run_git("reset", "-q", "--hard", "v0.1.0")
        self.assert_blocked("No commits since v0.1.0")

    def test_existing_next_tag_blocks(self):
        self.run_git("tag", "v0.1.1", "v0.1.0")
        result = self.bump("patch")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Newest release tag reachable from HEAD is v0.1.1", result.stderr)
        self.assert_clean()

    def test_prerelease_version_blocks(self):
        self.write_release_files("0.1.0-rc.1", changelog="# Changelog\n\n## 0.1.0-rc.1 — 2026-01-01\n\n- Candidate.\n")
        self.run_git("commit", "-q", "-am", "Candidate")
        self.run_git("tag", "v0.1.0-rc.1")
        (self.repo / "src.txt").write_text("more\n")
        self.run_git("commit", "-q", "-am", "More")
        result = self.bump("patch")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("pre-release and build suffixes are unsupported", result.stderr)
        self.assert_clean()

    def test_unreleased_heading_blocks(self):
        (self.repo / "CHANGELOG.md").write_text("# Changelog\n\n## 0.1.0 — Unreleased\n\n- Initial release.\n")
        self.run_git("commit", "-q", "-am", "Undated changelog")
        self.assert_blocked("must be '## 0.1.0 — YYYY-MM-DD'")

    def test_existing_next_section_blocks(self):
        (self.repo / "CHANGELOG.md").write_text("# Changelog\n\n## 0.1.0 — 2026-01-01\n\n- Initial.\n\n## 0.1.1 — 2025-12-01\n\n- Odd.\n")
        self.run_git("commit", "-q", "-am", "Odd changelog")
        self.assert_blocked("already has a section for 0.1.1")

    def test_cliff_failure_restores_files(self):
        self.env["RELEASE_TEST_FAIL_CLIFF"] = "yes"
        self.assert_blocked("Restored Cargo.toml, Cargo.lock and CHANGELOG.md")
        self.assertIn(["cargo", "update", "--workspace"], self.calls())

    def test_update_failure_restores_files(self):
        self.env["RELEASE_TEST_FAIL_UPDATE"] = "yes"
        self.assert_blocked("Restored Cargo.toml, Cargo.lock and CHANGELOG.md")
        self.assertFalse(any(c[0] == "git-cliff" for c in self.calls()))

    def test_empty_cliff_output_restores_files(self):
        self.env["RELEASE_TEST_CLIFF_OUTPUT"] = ""
        self.assert_blocked("unexpected git-cliff output: <empty>")

    def test_wrong_version_in_cliff_output_restores_files(self):
        self.env["RELEASE_TEST_CLIFF_OUTPUT"] = "## 9.9.9 — 2026-01-02\n\n- Add thing.\n"
        self.assert_blocked("unexpected git-cliff output: ## 9.9.9")

    def test_cliff_output_without_entries_restores_files(self):
        self.env["RELEASE_TEST_CLIFF_OUTPUT"] = "## 0.1.1 — 2026-01-02\n"
        self.assert_blocked("git-cliff produced no entries")


if __name__ == "__main__":
    unittest.main()
