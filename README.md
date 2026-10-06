# hevc-wasm (experimental)

A software decoder for a High Performance Mac's HEVC, for
[remotex](https://github.com/andrewtheguy/remotex)'s browser client where the
browser's `VideoDecoder` does not take HEVC Range Extensions 4:4:4 — Chrome on a
GPU without it, for one. It is FFmpeg's libavcodec, configured down to the HEVC
decoder, compiled to WebAssembly with Emscripten, behind the small C surface in
`src/decoder.c`.

A release is `hevc-wasm-vX.Y.Z.tar.gz`, holding the two files the page loads:
`hevc.js`, Emscripten's ES module glue, and `hevc.wasm`. This repository
publishes the source of the build and no binary of it: releases go to the
private [andrewtheguy/hevc-wasm-archives](https://github.com/andrewtheguy/hevc-wasm-archives),
for whoever can see it. No remotex build holds it either: remotex pins one by
version and SHA-256 (`src/hevc_wasm.rs`), an operator downloads that archive
from there and names it in the gateway's `[hevc_wasm]` table, and the
gateway reads it at start-up and serves its two files at `/hevc/`;
the page runs it in a worker of its own, shaped as a `VideoDecoder`
(remotex's `frontend/src/hevcWasmDecoder.ts` and `hevcWasm.worker.ts`).

## Building

```sh
./build.sh
```

Docker (or `CONTAINER=podman`) runs the pinned `emscripten/emsdk:6.0.10` image;
nothing else is needed on the host. FFmpeg is
[andrewtheguy/FFmpeg](https://github.com/andrewtheguy/FFmpeg/tree/remotex-wasm)'s
`remotex-wasm` branch: FFmpeg's release tag with this decoder's SIMD128 kernels
as one commit on top, tagged `n9.0.2-remotex.1`. The script downloads that
commit's archive, checks its SHA-256, and writes `build/out/hevc.js`,
`build/out/hevc.wasm`, `build/out/bench.js`, and the release archive in `dist/`.
A kernel changes on the fork's branch, which is rebased onto FFmpeg's next
release tag when it moves, and reaches here as a new commit and checksum in
`build.sh`.

The archive is reproducible, so `dist/`'s is byte-identical to the release's and
remotex serves it as the release. A changed build has another SHA-256, which
remotex refuses until its pin names it:

```toml
[hevc_wasm]
enabled = true
archive = "/path/to/hevc-wasm/dist/hevc-wasm-v0.0.1.tar.gz"
```

## Testing

```sh
bun install
bun test
bun run typecheck
```

The tests load `build/out/hevc.js` (or `$HEVC_WASM_DIR/hevc.js`) under Bun as
the page loads it, pthread pool and all, and feed it access units one at a time.
Their streams are committed in `test/data`: 4:2:0, 4:2:2 and 4:4:4, cropped
sizes, wavefront rows with 16-pixel CTBs, multiple slices, 10-bit, and color
descriptions. Every picture must match native FFmpeg's `-f framemd5`, recorded
in `test/data/reference.json`, at 1, 4 and 8 threads. They also cover
parameter-set-only units, size and format changes mid-stream, empty and garbage
units, joining mid-stream, and two decoders sharing one module.

A run needs no ffmpeg. `bun run fixtures` regenerates the streams and their
reference with the host's ffmpeg and libx265 from the specs in
`test/fixtures.ts`; commit what it writes.

## Releasing

Bump `VERSION`, commit, push, and run `./publish-private.sh`, logged in to `gh`
with an account that can write to hevc-wasm-archives. It builds `git archive HEAD`
on this machine, not in a workflow, since a public repository's workflow
artifacts are anyone's to download; attaches the archive and its `SHA256SUMS` to
hevc-wasm-archives' release `vX.Y.Z`; and tags the commit `vX.Y.Z` here. remotex
then takes it as a new version and checksum in `src/hevc_wasm.rs`.

## What is optimized

- **Slice threads, with wavefront parallel processing.** The Mac's stream sets
  `entropy_coding_sync_enabled_flag`, so a picture's CTB rows decode in parallel.
  Frame threads would hold each picture behind the next ones, and a still desktop
  sends no next one, so the decoder runs slice threads only and returns every
  picture from the unit that completed it. The threads are pthread workers the
  module starts before it resolves, as many as the page instantiates it with
  (remotex asks for `min(hardwareConcurrency, 8)`).
- **SIMD128.** FFmpeg's own WebAssembly kernels (IDCT, SAO), and the fork's for
  the rest of what the Mac's stream spends time in: the whole-pel block copy,
  the luma and chroma interpolation filters (horizontal, vertical and both) for
  uni-prediction, and the residual add. Every other loop gets the autovectorizer.
- **The arch traits configure leaves off for wasm**: unaligned loads, 64-bit
  integers and a count-leading-zeros instruction, which choose libavcodec's
  faster bitstream-reading paths.
- `-O3` with link-time optimization across libavcodec and the shim, bulk
  memory, non-trapping float-to-int, mimalloc for the threads' allocations.
- **One copy per picture.** `hevc_picture` returns each plane's address and
  stride in linear memory, so the page builds its `VideoFrame` straight over
  them and the constructor's copy is the only one.

## Measured

On an M2 Max, in Node 24, a High Performance Mac's stream captured with
remotex's `tests/hp_capture.sh`: 2880×1800, 4:4:4, 1039 pictures, about
53 Mbit/s, with animation playing. Milliseconds per picture:

| | 1 thread | 8 threads |
|---|---|---|
| FFmpeg defaults for wasm | 50.3 | 9.5 |
| this build | 44.2 (p95 52.6) | 8.9 (p95 12.8) |
| native FFmpeg 9 (NEON), for scale | 31.1 | 6.3 |

Every picture is bit-identical to native FFmpeg's (`ffmpeg -f framemd5`), on the
capture and on synthetic 4:2:0 and 4:4:4 streams of odd sizes. In headless
Chrome against a virtual Mac at 2x, remotex's paint worker's draw time (decode,
transfer and draw) was 11 ms at the median and 12 at p95, with no keyframe
requested.

What is left is entropy decoding: CABAC residual coding is over half the decode
time, serial within a row, and spread across the threads only by the wavefront.

```sh
node build/out/bench.js capture.h265 8
```

prints a digest per picture on stdout, to compare with
`ffmpeg -threads 1 -i capture.h265 -f framemd5 -`, and the timing on stderr.

## The pure-Rust decoder (experimental)

`rust/` holds a second decoder, written in Rust for the one shape of stream the
Mac sends and nothing else of HEVC: 4:4:4 at 8 bits, one slice per picture with
its coding tree block rows coded as a wavefront, I and P pictures with
short-term references. Any other stream is refused by name (tiles, B pictures,
weighted prediction, PCM, scaling lists, transform skip, long-term references,
other chroma formats or bit depths). Its decoding arithmetic was transcribed
from [rusty_h265](https://github.com/Remade-With-Rust/rusty_h265) 0.6.0
(Apache-2.0, `rust/hevc/LICENSE`), which decodes 4:2:0 only; the 4:4:4 handling,
the byte planes and the threading are this repository's.

- `rust/hevc` is the decoder: one access unit in, its picture out as three
  planes of bytes at the coded size, with the window to show and the colour the
  stream states. A picture's rows decode on rayon's pool, each two coding tree
  blocks behind the row above, and each row's thread deblocks a block behind
  its decoding and applies SAO, in place, a block behind that, as FFmpeg's
  does: the filters run while the block is still in cache, and nothing waits
  for a pass over the whole picture. The sample loops are SIMD128
  (`core::arch::wasm32`): the interpolation filters, which write samples
  straight from their last pass, block copies, the residual add, planar and
  angular intra prediction, SAO and a 16-bit inverse transform by dot
  products; coefficients are scaled as they are parsed, and CABAC keeps its
  registers in locals for a whole residual block.
- `rust/hevc-web` is the page's module, in the shape of remotex's `egfx` one:
  wasm-bindgen, a pool whose threads are seats the page's workers take
  (`runPoolThread`, `startPool`), and a `Decoder` with `input`, `decode` and
  `picture`. `picture` describes the picture in the same sixteen numbers
  `hevc_picture` does, so the paint worker uploads the planes to WebGL from the
  module's shared memory as it does now (remotex's `hevcPicture.ts`), and only the
  loader differs from the FFmpeg module's.
- `rust/hevc-bench` decodes a file natively and prints each picture's MD5 as
  `ffmpeg -f framemd5` does: `cargo run --release -p hevc-bench -- FILE THREADS [REPEATS]`
  from `rust/`. To profile the module itself, build it with
  `wasm-pack build rust/hevc-web --profiling --target web --no-pack --out-dir ../../tmp/pkg-prof`,
  which keeps the function names, and run it under `node --perf-basic-prof`
  and `perf record`; profiles and logs go under `tmp/`.

```sh
./build-rust.sh              # rust/hevc-web/pkg, on the pinned nightly, via wasm-pack
bun test test/rust.test.ts   # the mac-* fixtures, bit for bit against FFmpeg
```

It decodes every picture of the seven High Performance Mac captures tried
(2,833 pictures, 1600×1000 and 1600×256, including the long-term-refresh ones)
and of a 1080p camera video x265 encoded in the Mac's shape (2,896 pictures)
bit-identically to FFmpeg, natively on one thread and on six, and as the
WebAssembly module on one and four. The `mac-*` fixtures in `test/data` are
x265's nearest shape to the Mac's.

Measured against the FFmpeg module under Node on a six-core x86 workstation
that was busy with other work, so the two modules ran alternately on the same
four cores and the medians of three rounds are shown. The instruction and cycle
counts are `perf stat`'s for the whole process and do not depend on the load
(the FFmpeg bench hashes every picture of its first pass, so its counts are a
two-pass run less a one-pass one). Per picture:

| 1600×1000 Mac capture, 236 pictures | this decoder | FFmpeg module |
|---|---|---|
| 1 thread, median ms | 26.5 | 33.8 |
| 1 thread, M cycles | 77 | 90 |
| 4 threads, median ms | 9.4 | 10.9 |
| 4 threads, M cycles | 79 | 91 |
| M instructions | 195 | 213 |

| 1080p video in the Mac's shape, 2,896 pictures | this decoder | FFmpeg module |
|---|---|---|
| 1 thread, median ms | 6.6 | 7.1 |
| 1 thread, M cycles | 23 | 24 |
| 4 threads, median ms | 3.0 | 3.5 |
| 4 threads, M cycles | 25 | 25 |
| M instructions | 48 | 54 |

On the Mac's dense screen content this decoder is a seventh ahead in cycles,
and both spend their time where the stream does: CABAC residual parsing and
the 32×32 inverse transforms. On ordinary video, where motion compensation
dominates, the two are level, and the whole-pel block copies lead the profile
of each. Running the filters inside the wavefront rather than as passes over
the whole picture was worth a fifth of the cycles on four threads, where the
passes had left the threads spinning at barriers; SAO in place rather than
into a second picture, with the interpolation filters writing samples from
their last pass, another tenth on one thread. Natively the decoder runs its
scalar fallbacks, since the kernels are written for wasm32.

To benchmark on a busy host, pin both runs to the same cores (`taskset`),
alternate them, and read `perf stat -e instructions:u,cycles:u` rather than
wall time; wall-time medians drift with the load, the counts do not.

## Requirements

The page must be cross-origin isolated for `SharedArrayBuffer`, the threads'
shared memory: remotex's gateway serves every file with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` when `[hevc_wasm]` is enabled. The
browser must run shared-memory SIMD WebAssembly and build an I444 `VideoFrame`.
