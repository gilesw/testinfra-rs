#!/usr/bin/env bash
# Publish exactly the version committed and tagged at HEAD. Never edits files.
set -euo pipefail

die() { printf '%s\n' "error: $*" >&2; exit 1; }

mode=dry-run
tag=
for arg in "$@"; do
    case "$arg" in
        --publish) mode=publish ;;
        --help|-h)
            printf '%s\n' 'Usage: scripts/publish.sh [vVERSION] [--publish]' \
                'Defaults to a dry run. With no tag argument, uses the unique v* tag at HEAD.'
            exit 0
            ;;
        v*) [[ -z "$tag" ]] || die 'Specify only one tag'; tag=$arg ;;
        *) die "Unknown argument: $arg" ;;
    esac
done

cd "$(dirname "${BASH_SOURCE[0]}")/.."
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die 'Run from a Git checkout'
commit=$(git rev-parse --verify HEAD 2>/dev/null) || die 'Commit the release first'
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || die 'Working tree must be clean, including untracked files'

if [[ -z "$tag" ]]; then
    tag=$(git tag --points-at HEAD --list 'v*')
    [[ -n "$tag" && "$tag" != *$'\n'* ]] || die 'HEAD must have exactly one v* tag, or pass the release tag explicitly'
fi
git check-ref-format "refs/tags/$tag" >/dev/null || die 'Invalid tag name'
tag_commit=$(git rev-parse --verify "refs/tags/${tag}^{commit}" 2>/dev/null) || die "Tag does not exist: $tag"
[[ "$tag_commit" == "$commit" ]] || die "$tag does not point to HEAD"

# Parse Cargo's metadata rather than guessing at TOML syntax. This script is
# intentionally for a single-crate repository, not a multi-package release tool.
version=$(cargo metadata --no-deps --locked --format-version 1 | python3 -c '
import json, sys
from pathlib import Path
data = json.load(sys.stdin)
manifest = Path("Cargo.toml").resolve()
packages = [p for p in data["packages"] if Path(p["manifest_path"]).resolve() == manifest]
if len(packages) != 1:
    sys.exit("error: expected one root package")
p = packages[0]
if p["publish"] is not None and "crates-io" not in p["publish"]:
    sys.exit("error: Cargo.toml does not permit publication to crates.io")
print(p["version"])
')
[[ "$tag" == "v$version" ]] || die "Tag $tag must match Cargo.toml version v$version"

# Compare tag objects as well as their commits, including annotated tags.
# ls-remote is read-only and does not change the checkout or local refs.
verify_release() {
    [[ "$(git rev-parse HEAD)" == "$commit" ]] || die 'HEAD changed during release checks'
    [[ -z "$(git status --porcelain --untracked-files=all)" ]] || die 'Release checks changed the working tree'
    local local_ref remote_ref
    local_ref=$(git rev-parse --verify "refs/tags/$tag")
    [[ "$(git rev-parse --verify "refs/tags/${tag}^{commit}")" == "$commit" ]] || die 'Tag changed during release checks'
    remote_ref=$(git ls-remote --exit-code --refs origin "refs/tags/$tag") || die "Push $tag to origin first"
    [[ "$remote_ref" == "$local_ref"$'\t'"refs/tags/$tag" ]] || die 'Local and origin release tags differ'
}

verify_release
printf 'Checking %s at %s (%s).\n' "$tag" "$commit" "$mode"
mise run check
verify_release
cargo publish --dry-run --locked --registry crates-io

if [[ "$mode" == publish ]]; then
    verify_release
    # Cargo's token provider reads inherited CARGO_REGISTRY_TOKEN directly.
    # Do not expand the token into arguments or persist it with cargo login.
    cargo publish --locked --registry crates-io
else
    printf '%s\n' "Dry run passed. To upload this release: scripts/publish.sh $tag --publish"
fi
