#!/usr/bin/env bash
# Build the release archive on the operator's own machine, and put it where only
# collaborators can read it.
#
# Usage:
#   ./publish-private.sh
#
# This repository, which is public, publishes the source of the build and no
# binary of it. The archive goes to a release of ARCHIVES_REPO, a private
# repository, and remotex's operators download it from there. It is built here
# rather than in a workflow because a public repository's workflow artifacts can
# be downloaded by anyone with a GitHub account.
#
# Bump VERSION, commit and push first. What gets built is `git archive HEAD`, not
# this directory, so nothing uncommitted or ignored can reach the archive. The tag
# is `v$(cat VERSION)`, created twice: on the private repository, as the release
# holding the archive, and on this one, as the plain git tag of the commit it was
# built from.
set -euo pipefail

ARCHIVES_REPO=andrewtheguy/hevc-wasm-archives

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

[ $# -eq 0 ] || { sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2; }

version="$(cat VERSION)"
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || { echo "VERSION is not X.Y.Z: '$version'" >&2; exit 1; }
tag="v$version"
archive="hevc-wasm-$tag.tar.gz"

# The tag names a commit, so the archive must be that commit's and the commit must
# be one anybody can fetch.
[ -z "$(git status --porcelain)" ] \
  || { echo "the working tree has uncommitted changes; a release is of a commit" >&2; exit 1; }
sha="$(git rev-parse HEAD)"
git fetch --quiet --tags origin
[ -n "$(git branch -r --contains "$sha")" ] \
  || { echo "$sha is on no branch of origin; push it first" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "$tag already exists here; bump VERSION" >&2
  exit 1
fi

# Before the build rather than after it: minutes of compiling is a poor way to
# learn that `gh` is logged in to the wrong account.
gh release list --repo "$ARCHIVES_REPO" --limit 1 >/dev/null \
  || { echo "cannot read $ARCHIVES_REPO: gh auth login, with an account that has access" >&2; exit 1; }
if gh release view "$tag" --repo "$ARCHIVES_REPO" >/dev/null 2>&1; then
  echo "$ARCHIVES_REPO already has a release $tag; bump VERSION" >&2
  exit 1
fi

# The commit, and only the commit, with this checkout's wasm-pack.
work="$here/tmp/publish-$tag"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
git archive "$sha" | tar -x -C "$work"
ln -s "$here/node_modules" "$work/node_modules"
"$work/build.sh"

out="$work/dist"
[ -f "$out/$archive" ] || { echo "the build wrote no $archive" >&2; exit 1; }
# A corruption check, not a tamper check: it lives on the same release as the file
# it covers.
(cd "$out" && shasum -a 256 -- "$archive" >SHA256SUMS)
cat "$out/SHA256SUMS"

# Draft first, and published only after this repository's tag is pushed, so a
# failure before the last step leaves a deletable draft and never a release whose
# source tag does not exist.
echo ">> releasing $tag on $ARCHIVES_REPO (draft)"
gh release create "$tag" --repo "$ARCHIVES_REPO" --draft \
  --title "$tag" \
  --notes "Built from https://github.com/andrewtheguy/hevc-wasm/commit/$sha" \
  "$out/SHA256SUMS" "$out/$archive"

git tag "$tag" "$sha"
git push origin "refs/tags/$tag"

gh release edit "$tag" --repo "$ARCHIVES_REPO" --draft=false
echo ">> published $tag; remotex pins it in src/hevc_wasm.rs by $(cut -d' ' -f1 "$out/SHA256SUMS")"
