#!/usr/bin/env bash
# Build the HEVC WebAssembly decoder for remotex's page: rust/hevc-web, the `hevc`
# crate behind the few calls the page's decode worker makes, with threads and
# SIMD, through wasm-pack. Writes build/out/hevc.js (wasm-bindgen's ES module
# glue) and build/out/hevc.wasm, and the release archive
# dist/hevc-wasm-vX.Y.Z.tar.gz holding the two, X.Y.Z being hevc-web's version.
#
# The nightly rust/hevc-web/rust-toolchain.toml names is installed by rustup on
# the first build; wasm-pack comes from `bun install`, run here where it is
# missing.
#
#   ./build.sh
set -euo pipefail
cd "$(dirname "$0")"

version=$(cargo metadata --manifest-path rust/hevc-web/Cargo.toml --no-deps --offline --format-version 1 \
  | jq -r '.packages[] | select(.name == "hevc-web") | .version')
[ -x node_modules/.bin/wasm-pack ] || bun install --frozen-lockfile

rm -rf build/pkg build/out dist
node_modules/.bin/wasm-pack build rust/hevc-web --release --target web --no-pack \
  --out-name hevc --out-dir "$PWD/build/pkg"
mkdir -p build/out dist
cp build/pkg/hevc.js build/out/hevc.js
cp build/pkg/hevc_bg.wasm build/out/hevc.wasm

# The release: the two files the page loads, and nothing else. GNU tar with fixed
# names, owners and times, so one build's archive is byte for byte the next's
# from the same toolchain.
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 \
  -C build/out -cf - hevc.js hevc.wasm | gzip -9n >"dist/hevc-wasm-v$version.tar.gz"
ls -l build/out dist
