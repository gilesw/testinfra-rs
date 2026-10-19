#!/usr/bin/env bash
# Create and push the annotated release tag for the version committed at HEAD,
# which must be the default branch's tip on origin. Never edits files.
set -euo pipefail

die() { printf '%s\n' "error: $*" >&2; exit 1; }

push=1
for arg in "$@"; do
    case "$arg" in
        --no-push) push=0 ;;
        --help|-h)
            printf '%s\n' 'Usage: scripts/tag.sh [--no-push]' \
                'Tags HEAD as v<Cargo.toml version> and pushes the tag, which publishes. --no-push only tags.'
            exit 0
            ;;
        *) die "Unknown argument: $arg" ;;
    esac
done

cd "$(dirname "${BASH_SOURCE[0]}")/.."
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die 'Run from a Git checkout'
commit=$(git rev-parse --verify HEAD 2>/dev/null) || die 'Commit the release first'
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || die 'Working tree must be clean, including untracked files'

# Releases are cut from the default branch only, so a tag never publishes an
# unmerged commit from a pull-request branch.
default=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null) \
    || die "origin has no default branch recorded; run 'git remote set-head origin --auto'"
default=${default#origin/}
branch=$(git symbolic-ref --quiet --short HEAD) || die "Detached HEAD; check out $default"
[[ "$branch" == "$default" ]] || die "On $branch; releases are tagged on $default after the release commit is merged"

# Parse Cargo's metadata rather than guessing at TOML syntax.
version=$(cargo metadata --no-deps --locked --format-version 1 | python3 -c '
import json, sys
from pathlib import Path
data = json.load(sys.stdin)
manifest = Path("Cargo.toml").resolve()
packages = [p for p in data["packages"] if Path(p["manifest_path"]).resolve() == manifest]
if len(packages) != 1:
    sys.exit("error: expected one root package")
print(packages[0]["version"])
')
tag=v$version
git check-ref-format "refs/tags/$tag" >/dev/null || die "Invalid tag name: $tag"
! git rev-parse -q --verify "refs/tags/$tag" >/dev/null || die "Tag $tag already exists locally"

# ls-remote is read-only. Exit 2 means the ref is absent; anything else is a connection problem.
set +e
remote=$(git ls-remote --exit-code --refs origin "refs/heads/$default" "refs/tags/$tag" 2>&1)
status=$?
set -e
(( status == 0 || status == 2 )) || die "Cannot query origin: $remote"
! grep -q "refs/tags/$tag\$" <<<"$remote" || die "Tag $tag already exists on origin"
remote_head=$(awk -v ref="refs/heads/$default" '$2 == ref { print $1 }' <<<"$remote")
[[ -n "$remote_head" ]] || die "origin has no branch $default"
[[ "$remote_head" == "$commit" ]] || die "HEAD is not the tip of origin/$default; merge the release commit and pull first"

python3 - "$version" <<'PY'
import re, sys
from pathlib import Path
version = sys.argv[1]
headings = [l for l in Path("CHANGELOG.md").read_text().splitlines() if l.startswith("## ")]
if not headings or not re.fullmatch(rf"## {re.escape(version)} — \d{{4}}-\d{{2}}-\d{{2}}", headings[0]):
    found = headings[0] if headings else "<none>"
    sys.exit(f"error: the first CHANGELOG.md section must be '## {version} — YYYY-MM-DD', found '{found}'")
PY

git tag -a "$tag" -m "Release $version"
push_cmd="git push origin refs/tags/$tag"
if (( push )); then
    $push_cmd || die "Push rejected; $tag remains local. Retry with: $push_cmd"
    printf 'Pushed %s. The release workflow now publishes %s.\n' "$tag" "$version"
else
    printf 'Created %s at %s. Pushing the tag publishes the release:\n  %s\n' "$tag" "$commit" "$push_cmd"
fi
