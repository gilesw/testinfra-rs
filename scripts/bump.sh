#!/usr/bin/env bash
# Raise the crate version by one level and insert the changelog section for it.
# Edits Cargo.toml, Cargo.lock and CHANGELOG.md; never commits, tags or pushes.
set -euo pipefail

die() { printf '%s\n' "error: $*" >&2; exit 1; }

level=
for arg in "$@"; do
    case "$arg" in
        major|minor|patch) [[ -z "$level" ]] || die 'Specify only one level'; level=$arg ;;
        --help|-h)
            printf '%s\n' 'Usage: scripts/bump.sh major|minor|patch' \
                'Bumps Cargo.toml and Cargo.lock and inserts a CHANGELOG.md section rendered by git-cliff.'
            exit 0
            ;;
        *) die "Unknown argument: $arg" ;;
    esac
done
[[ -n "$level" ]] || die 'Specify the level: major, minor or patch'

cd "$(dirname "${BASH_SOURCE[0]}")/.."
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die 'Run from a Git checkout'
git rev-parse --verify HEAD >/dev/null 2>&1 || die 'Commit something first'
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || die 'Working tree must be clean, including untracked files'
[[ -f cliff.toml ]] || die 'cliff.toml is missing'
command -v git-cliff >/dev/null || die 'git-cliff is not installed; run mise install'

# Parse Cargo's metadata rather than guessing at TOML syntax.
read_version() {
    cargo metadata --no-deps --locked --format-version 1 | python3 -c '
import json, sys
from pathlib import Path
data = json.load(sys.stdin)
manifest = Path("Cargo.toml").resolve()
packages = [p for p in data["packages"] if Path(p["manifest_path"]).resolve() == manifest]
if len(packages) != 1:
    sys.exit("error: expected one root package")
print(packages[0]["version"])
'
}

current=$(read_version)
next=$(python3 - "$current" "$level" <<'PY'
import re, sys
current, level = sys.argv[1:]
m = re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", current)
if not m:
    sys.exit(f"error: {current} is not plain MAJOR.MINOR.PATCH; pre-release and build suffixes are unsupported")
major, minor, patch = map(int, m.groups())
if level == "major":
    major, minor, patch = major + 1, 0, 0
elif level == "minor":
    minor, patch = minor + 1, 0
else:
    patch += 1
print(f"{major}.{minor}.{patch}")
PY
)
prev_tag=v$current
next_tag=v$next

git rev-parse -q --verify "refs/tags/${prev_tag}^{commit}" >/dev/null || die "Tag $prev_tag does not exist; release $current before bumping"
git merge-base --is-ancestor "$prev_tag" HEAD || die "$prev_tag is not an ancestor of HEAD"
newest=$(git tag --sort=-v:refname --merged HEAD --list 'v[0-9]*' | head -n1)
[[ "$newest" == "$prev_tag" ]] || die "Newest release tag reachable from HEAD is $newest, not $prev_tag"
[[ "$(git rev-list --count "$prev_tag..HEAD")" -gt 0 ]] || die "No commits since $prev_tag; nothing to release"
! git rev-parse -q --verify "refs/tags/$next_tag" >/dev/null || die "Tag $next_tag already exists"

python3 - "$current" "$next" <<'PY'
import re, sys
from pathlib import Path
current, nxt = sys.argv[1:]
headings = [l for l in Path("CHANGELOG.md").read_text().splitlines() if l.startswith("## ")]
if not headings:
    sys.exit("error: CHANGELOG.md has no release sections")
if not re.fullmatch(rf"## {re.escape(current)} — \d{{4}}-\d{{2}}-\d{{2}}", headings[0]):
    sys.exit(f"error: the first CHANGELOG.md section must be '## {current} — YYYY-MM-DD', found '{headings[0]}'")
if any(h.startswith(f"## {nxt} ") or h == f"## {nxt}" for h in headings):
    sys.exit(f"error: CHANGELOG.md already has a section for {nxt}")
PY

# Every write below is undone on failure; the tree was clean, so HEAD is the baseline.
edited=0
restore() {
    if (( edited )); then
        git checkout --quiet -- Cargo.toml Cargo.lock CHANGELOG.md
        printf '%s\n' 'Restored Cargo.toml, Cargo.lock and CHANGELOG.md' >&2
    fi
}
trap '[[ $? -eq 0 ]] || restore' EXIT

printf 'Bumping %s -> %s (%s).\n' "$current" "$next" "$level"
edited=1
python3 - "$current" "$next" <<'PY'
import re, sys
from pathlib import Path
current, nxt = sys.argv[1:]
path = Path("Cargo.toml")
lines = path.read_text().splitlines(keepends=True)
table, hits = None, 0
for i, line in enumerate(lines):
    header = re.match(r"\s*\[([^\]]+)\]", line)
    if header:
        table = header.group(1).strip()
        continue
    m = re.match(r'(\s*version\s*=\s*")([^"]+)(".*)$', line, re.S)
    if table == "package" and m:
        if m.group(2) != current:
            sys.exit(f"error: Cargo.toml says {m.group(2)}, cargo metadata says {current}")
        lines[i] = f"{m.group(1)}{nxt}{m.group(3)}"
        hits += 1
if hits != 1:
    sys.exit("error: expected exactly one version line under [package]")
path.write_text("".join(lines))
PY

cargo update --workspace
git diff --quiet -- Cargo.lock && die 'cargo update --workspace did not change Cargo.lock'
verified=$(read_version)
[[ "$verified" == "$next" ]] || die "Cargo reports $verified, expected $next"

section=$(git-cliff --config cliff.toml --strip all --tag "$next_tag" "$prev_tag..HEAD")
python3 - "$next" "$section" <<'PY'
import re, sys
from pathlib import Path
version = sys.argv[1]
section = sys.argv[2].strip()
lines = section.splitlines()
if not lines or not re.fullmatch(rf"## {re.escape(version)} — \d{{4}}-\d{{2}}-\d{{2}}", lines[0]):
    sys.exit("error: unexpected git-cliff output: " + (lines[0] if lines else "<empty>"))
if not any(l.startswith("- ") for l in lines[1:]):
    sys.exit("error: git-cliff produced no entries")
path = Path("CHANGELOG.md")
head, sep, rest = path.read_text().partition("\n## ")
if not sep:
    sys.exit("error: CHANGELOG.md has no release sections")
path.write_text(head.rstrip("\n") + "\n\n" + section + "\n\n## " + rest)
PY

git status --short
printf '%s\n' "Prepared $next. Next: 'mise run check', 'git commit -am \"Release $next\"', 'mise run tag'."
