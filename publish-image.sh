#!/usr/bin/env bash
# Put every release archive of ARCHIVES_REPO in one image, and push it where only
# the readers of that repository can pull it: IMAGE, tagged with the newest
# release's version, as hevc-wasm's versions run and not zDesk's.
#
# Usage:
#   ./publish-image.sh
#
# It is how a Kubernetes deployment of zDesk is given the decoder without an
# image of zDesk's own: the pod runs zDesk's public image and mounts this one
# as an image volume on the release tree's share/zdesk, where the gateway looks
# for exactly the archive it pins. The deployment names the tag, and changes it
# only when zDesk pins a newer hevc-wasm. Every release up to that one is in
# it, so a zDesk release that pins an older one finds it there too.
# It is run on its own, from a machine with podman, after a release that a
# deployment is to mount; publish-private.sh does not run it.
#
# The image is the archives at its root and nothing else. Its label links the
# package to ARCHIVES_REPO, and the package must stay private: this asks ghcr
# whether an anonymous client can pull it, does not push unless the answer is no
# or there is no package yet, and fails unless it is no after the push.
#
# Log in to ghcr first, with a token that has write:packages, and to GitHub with
# access to ARCHIVES_REPO:
#
#   gh auth token | podman login ghcr.io -u <user> --password-stdin
set -euo pipefail

ARCHIVES_REPO=andrewtheguy/hevc-wasm-archives
REGISTRY=ghcr.io
PACKAGE=andrewtheguy/hevc-wasm-archives
IMAGE="$REGISTRY/$PACKAGE"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

[ $# -eq 0 ] || { sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2; }

# Before the downloads rather than after them.
podman login --get-login "$REGISTRY" >/dev/null 2>&1 \
  || { echo "not logged in to $REGISTRY: podman login $REGISTRY" >&2; exit 1; }

# Newest first, as GitHub lists them; drafts are not releases yet.
mapfile -t tags < <(gh release list --repo "$ARCHIVES_REPO" --exclude-drafts --limit 1000 \
  --json tagName --jq '.[].tagName')
[ "${#tags[@]}" -gt 0 ] || { echo "$ARCHIVES_REPO has no releases" >&2; exit 1; }
newest="${tags[0]}"

work="$here/tmp/image"
rm -rf "$work"
mkdir -p "$work/archives"
trap 'rm -rf "$work"' EXIT

for tag in "${tags[@]}"; do
  archive="hevc-wasm-$tag.tar.gz"
  echo ">> downloading $archive"
  gh release download "$tag" --repo "$ARCHIVES_REPO" --dir "$work/$tag" \
    --pattern "$archive" --pattern SHA256SUMS
  (cd "$work/$tag" && shasum -a 256 --check --quiet SHA256SUMS) \
    || { echo "$archive does not match its release's SHA256SUMS" >&2; exit 1; }
  mv "$work/$tag/$archive" "$work/archives/"
done

cat >"$work/Containerfile" <<CONTAINERFILE
FROM scratch
LABEL org.opencontainers.image.source=https://github.com/$ARCHIVES_REPO
LABEL org.opencontainers.image.description="hevc-wasm's release archives, $newest and every one before it"
COPY archives/ /
CONTAINERFILE

echo ">> building $IMAGE:$newest"
podman build --platform linux/amd64 -f "$work/Containerfile" \
  -t "$IMAGE:$newest" "$work"

# What ghcr tells a client with no credentials that asks to pull the package, as
# measured: a token for a public package, 401 UNAUTHORIZED for a private one, and
# 403 DENIED for one that does not exist. Anything else says nothing.
anonymous_access() {
  local response body code
  response="$(curl -sS -w '\n%{http_code}' "https://$REGISTRY/token?scope=repository:$PACKAGE:pull")" \
    || { echo "unreachable"; return; }
  body="${response%$'\n'*}"
  code="${response##*$'\n'}"
  case "$code" in
    200) grep -q '"token":"[^"]' <<<"$body" && echo public || echo "HTTP 200 without a token" ;;
    401) grep -q '"code":"UNAUTHORIZED"' <<<"$body" && echo private || echo "HTTP 401: $body" ;;
    403) grep -q '"code":"DENIED"' <<<"$body" && echo missing || echo "HTTP 403: $body" ;;
    *) echo "HTTP $code: $body" ;;
  esac
}

# Before anything goes up. A package that does not exist yet is the first push,
# which the check after it covers.
access="$(anonymous_access)"
case "$access" in
  private | missing) ;;
  public) echo "$IMAGE is public: make the package private before pushing to it" >&2; exit 1 ;;
  *) echo "could not tell whether $IMAGE is private ($access); not pushing" >&2; exit 1 ;;
esac

echo ">> pushing $IMAGE:$newest"
podman push "$IMAGE:$newest"

access="$(anonymous_access)"
[ "$access" = private ] \
  || { echo "$IMAGE is not confirmed private after the push ($access): make the package private" >&2; exit 1; }

echo ">> pushed $IMAGE:$newest, ${#tags[@]} archives; anonymous pull refused"
