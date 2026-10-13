# hevc-wasm (experimental)

A software decoder for a High Performance Mac's HEVC, for
[remotex](https://github.com/andrewtheguy/remotex)'s browser client where the
browser's `VideoDecoder` does not take HEVC Range Extensions 4:4:4 — Chrome on a
GPU without it, for one. It is written in Rust for the one shape of stream the
Mac sends and nothing else of HEVC, compiled to WebAssembly with SIMD128 and
threads, and decodes bit for bit as FFmpeg does.

A release is `hevc-wasm-vX.Y.Z.tar.gz`, holding the two files the page loads:
`hevc.js`, wasm-bindgen's ES module glue, and `hevc.wasm`. This repository
publishes the source of the build and no binary of it: releases go to the
private [andrewtheguy/hevc-wasm-archives](https://github.com/andrewtheguy/hevc-wasm-archives),
for whoever can see it. No remotex build holds it either, since its licence
keeps it out of every remotex artifact:
remotex pins one by version and SHA-256 (`src/hevc_wasm.rs`), an operator
downloads that archive from there and names it in the gateway's `[hevc_wasm]`
table, and the gateway reads it at start-up and serves its two files at
`/hevc/`; the page runs it in a worker of its own, shaped as a `VideoDecoder`,
with the threads as workers of the page's bundle (remotex's
`frontend/src/hevcWasmDecoder.ts`, `hevcWasm.worker.ts` and
`hevcPool.worker.ts`).

## Building

```sh
bun install
./build.sh
```

wasm-pack comes from `bun install`, and the compiler is the nightly
`rust/hevc-web/rust-toolchain.toml` names, which rustup installs on the first
build. A nightly because the module's threads share a memory, and the standard
library as shipped for `wasm32-unknown-unknown` is built without `atomics`: it
links, but its locks are single-thread stubs, and only a nightly Cargo builds
it with them (`build-std`, in `rust/hevc-web/.cargo/config.toml`). The pin is
the channel, not a date: the decoder itself builds on stable. The script writes
`build/out/hevc.js`, `build/out/hevc.wasm`, and the release archive in
`dist/`.

The archive is built with fixed names, owners and times, so on the same
toolchain one build's archive is byte for byte the next's, and remotex serves
it as the release. A changed build has another SHA-256, which remotex refuses
until its pin names it:

```toml
[hevc_wasm]
enabled = true
archive = "/path/to/hevc-wasm/dist/hevc-wasm-v0.0.9.tar.gz"
```

## Testing

```sh
bun install
bun test
bun run typecheck
```

The tests load `build/out/hevc.js` and `hevc.wasm` (or `$HEVC_WASM_DIR`'s)
under Bun as the page loads them: the module on a shared memory, its pool's
threads as workers that each run an instance of it. The `mac-*` streams in
`test/data`, x265's nearest shape to the Mac's, must decode to native FFmpeg's
`-f framemd5`, recorded in `test/data/reference.json`, on one thread and on
four; the other streams there, of other shapes, must be refused by name. The
tests also cover where the planes are in the memory, joining a stream before
its keyframe, two decoders side by side, garbage units, and a display put
together from its strips.

A run needs no ffmpeg. `bun run fixtures` regenerates the streams and their
reference with the host's ffmpeg and libx265 from the specs in
`test/fixtures.ts`; commit what it writes.

The decoder must return an error on any input and never trap, since a trap
takes the module down for every decoder in it. `rust/hevc/fuzz` holds a
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) target that feeds a
stream access unit by access unit to two decoders, one on the calling thread
and one on a pool of three, and requires the same picture or the same
failure of both. It is built natively, with the sanitizer and the debug
assertions on, so that it catches a bounds the rows' shared accessors take
on trust and a race in the wavefront as well as a panic. The SIMD loops are
the module's alone (natively the decoder runs their scalar counterparts), so
`bun test` also feeds damaged copies of the fixtures to the module, on one
thread and on four, and requires that nothing traps and that the two agree.

```sh
cd rust/hevc
mkdir -p fuzz/corpus/decode && cp ../../test/data/*.h265 fuzz/corpus/decode/
cargo +nightly fuzz run decode -- -max_len=65536 -jobs=5 -workers=5
```

An input that fails lands in `fuzz/artifacts/decode/`; `cargo +nightly fuzz
run decode fuzz/artifacts/decode/<file>` replays it.

## Releasing

Bump the version in `rust/hevc-web/Cargo.toml` (the module's crate: the archive
and the tag take their number from it, through `cargo metadata` and `jq`),
commit, push, and run `./publish-private.sh`, logged in to `gh`
with an account that can write to hevc-wasm-archives. It builds `git archive HEAD`
on this machine, not in a workflow, since a public repository's workflow
artifacts are anyone's to download; attaches the archive and its `SHA256SUMS` to
hevc-wasm-archives' release `vX.Y.Z`; tags the commit `vX.Y.Z` here, with a
release of the source alone whose notes give the archive's digest. remotex then
takes it as a new version and checksum in `src/hevc_wasm.rs`.

## The decoder

`rust/` holds the decoder, written for the one shape of stream the Mac sends
and nothing else of HEVC: 4:4:4 at 8 bits, one slice per picture with its
coding tree block rows coded as a wavefront, I and P pictures with short-term
references. Any other stream is refused by name (HEVC's tiles, B pictures,
weighted prediction, PCM, scaling lists, transform skip, long-term references,
other chroma formats or bit depths). A display the Mac sends as four strips
(`tilesPerFrame` 4 in remotex's offer) is that same shape, each picture a strip
of the display, and the decoder puts the display together from them. Its decoding arithmetic was transcribed from
[rusty_h265](https://github.com/Remade-With-Rust/rusty_h265) 0.6.0
(Apache-2.0, `rust/hevc/LICENSE`), which decodes 4:2:0 only; the 4:4:4 handling,
the byte planes and the threading are this repository's.

- `rust/hevc` is the decoder: one access unit in, its picture out as three
  planes of bytes at the coded size, with the window to show and the colour the
  stream states. A picture's rows decode on rayon's pool, each two coding tree
  blocks behind the row above, and each row's thread deblocks a block behind
  its decoding and applies SAO, in place, a block behind that, as FFmpeg's
  does: the filters run while the block is still in cache, and nothing waits
  for a pass over the whole picture. A P picture's rows start as the first
  reference's, so the still blocks that are most of a screen are already in
  place, and a buffer that held the picture before the reference, of a
  display in strips the same strip two frames back, is taken for it so that
  only the blocks changed since are copied. The sample loops are SIMD128
  (`core::arch::wasm32`): the interpolation filters, which write samples
  straight from their last pass, block copies, the residual add, planar and
  angular intra prediction, SAO and a 16-bit inverse transform by dot
  products, over the coded columns alone of a block with few coefficients;
  coefficients are scaled as they are parsed, and CABAC keeps its two
  registers in locals and its LPS table packed by model, the walk over a
  block's sub-blocks and each coded sub-block decoding in functions of
  their own, the significance contexts from a table built at compile time
  and the flags unrolled by position.
- `rust/hevc-web` is the page's module, in the shape of remotex's `egfx` one:
  wasm-bindgen, a pool whose threads are seats the page's workers take
  (`runPoolThread`, `startPool`), and a `Decoder` with `input`, `decode`,
  `decodeStrip` for a display in strips, and `picture`. `picture` describes the picture in sixteen numbers: its size, the
  chroma layout, the colour the stream states, each plane's address and
  stride in the module's memory, and whether it is a keyframe, so the paint
  worker uploads the planes to WebGL from the module's shared memory
  (remotex's `hevcPicture.ts`).
- `rust/hevc-bench` decodes a file natively and prints each picture's MD5 as
  `ffmpeg -f framemd5` does: `cargo run --release -p hevc-bench -- FILE THREADS [REPEATS]`
  from `rust/`. To profile the module itself, run it under
  `node --perf-prof` and `perf record`: the build keeps the function names.
  Profiles and logs go under `tmp/`.

It decodes every picture of the fourteen captures of a High Performance Mac's
1440×900 display in strips taken on 2026-10-12 (31,683 pictures, 1440×240 at
1x and 2880×464 at 2x), of the seven captures of whole pictures before them
(2,833 pictures, 1600×1000 and 1600×256, including the long-term-refresh
ones) and of a 1080p camera video x265 encoded in the Mac's shape (2,896
pictures) bit-identically to FFmpeg, as the WebAssembly module on four
threads. The `mac-*` fixtures in `test/data` are x265's nearest shape to the
Mac's.

On one thread it takes fewer cycles than native FFmpeg wherever most of a
picture is the picture before, up to two fifths fewer on a strip at 1x and
two thirds at 2x, and up to a fifth more where a picture is new, a video
over the whole display. The tables, how a change is measured (`bench/run.sh`) and
what each step that got here was worth are in
[docs/benchmarks.md](docs/benchmarks.md).

Natively the decoder runs its scalar fallbacks, since the kernels are written
for wasm32. How it is put together, and where its time goes, is in
[docs/architecture.md](docs/architecture.md); what it does not do yet, in
[docs/remaining.md](docs/remaining.md).

## Requirements

The page must be cross-origin isolated for `SharedArrayBuffer`, the threads'
shared memory: remotex's gateway serves every file with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` when `[hevc_wasm]` is enabled. The
browser must run shared-memory SIMD WebAssembly and build an I444 `VideoFrame`.
