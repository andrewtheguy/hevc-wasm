#!/usr/bin/env bash
# Build the pure-Rust HEVC decoder for the page: rust/hevc-web, with threads and
# SIMD, through wasm-pack into rust/hevc-web/pkg (hevc_web.js, hevc_web_bg.wasm).
# The pinned nightly in rust/hevc-web/rust-toolchain.toml is installed by rustup
# on the first build; wasm-pack comes from `bun install`.
#
#   ./build-rust.sh
set -euo pipefail
cd "$(dirname "$0")"
node_modules/.bin/wasm-pack build rust/hevc-web --release --target web --no-pack
ls -l rust/hevc-web/pkg
