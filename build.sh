#!/usr/bin/env bash
# Build the HEVC WebAssembly decoder: FFmpeg's libavcodec configured down to the
# HEVC decoder, with its SIMD128 kernels and slice threads, behind src/decoder.c.
#
# FFmpeg comes from andrewtheguy/FFmpeg's remotex-wasm branch, FFmpeg's release
# with this decoder's SIMD128 kernels as a commit of their own, pinned here by
# that commit and its archive's SHA-256. A kernel changes there, rebased onto a
# new release tag when FFmpeg moves, and reaches here as a new pin.
#
# Runs Emscripten in its pinned container, so nothing but Docker (or Podman, as
# CONTAINER=podman) is needed on the host. Writes build/out/hevc.js and hevc.wasm,
# build/out/bench.js, and the release archive dist/hevc-wasm-v$VERSION.tar.gz.
#
#   ./build.sh
set -euo pipefail

cd "$(dirname "$0")"

EMSDK_IMAGE=docker.io/emscripten/emsdk:6.0.10
# Tag n9.0.2-remotex.1: FFmpeg 9.0.2 and the kernels.
FFMPEG_COMMIT=5b81e18029e561cf13395588ae7353b4d68ec905
FFMPEG_SHA256=395fda1223dd224d3bd281bfc1ec4616012e9409a76b5d3b533e7d962d1c4c68
CONTAINER=${CONTAINER:-docker}
VERSION=$(cat VERSION)

mkdir -p build
tarball=build/ffmpeg-$FFMPEG_COMMIT.tar.gz
if [[ ! -f $tarball ]]; then
  curl -fsSL -o "$tarball.part" "https://github.com/andrewtheguy/FFmpeg/archive/$FFMPEG_COMMIT.tar.gz"
  mv "$tarball.part" "$tarball"
fi
echo "$FFMPEG_SHA256  $tarball" | shasum -a 256 -c -

"$CONTAINER" run --rm \
  -v "$PWD:/src" -w /src \
  -e FFMPEG_COMMIT="$FFMPEG_COMMIT" \
  -e VERSION="$VERSION" \
  -u "$(id -u):$(id -g)" \
  -e HOME=/src/build/home \
  -e EM_CACHE=/src/build/emcache \
  "$EMSDK_IMAGE" bash /src/build-in-container.sh

ls -l build/out dist
