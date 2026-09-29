# hevc-wasm (experimental)

A software decoder for a High Performance Mac's HEVC, for
[remotex](https://github.com/andrewtheguy/remotex)'s browser client where the
browser's `VideoDecoder` does not take HEVC Range Extensions 4:4:4 — Chrome on a
GPU without it, for one. It is FFmpeg's libavcodec, configured down to the HEVC
decoder, compiled to WebAssembly with Emscripten, behind the small C surface in
`src/decoder.c`.

A release is `hevc-wasm-vX.Y.Z.tar.gz`, holding the two files the page loads:
`hevc.js`, Emscripten's ES module glue, and `hevc.wasm`. remotex's non-default
`hevc-wasm` feature pins one by version and SHA-256 and serves it at `/hevc/`;
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

To try a local build in remotex without a release:

```sh
REMOTEX_HEVC_WASM_DIR=../hevc-wasm/build/out cargo build --profile qa --features hevc-wasm
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

Bump `VERSION`, push, and run the Publish workflow (`gh workflow run publish.yml`).
It builds on GitHub's runner and attaches the archive and its `SHA256SUMS` to the
release `vX.Y.Z`. remotex then takes it as a new version and checksum in its
`build.rs`.

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

## Requirements

The page must be cross-origin isolated for `SharedArrayBuffer`, the threads'
shared memory: remotex's gateway serves every file with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` when built with the feature. The
browser must run shared-memory SIMD WebAssembly and build an I444 `VideoFrame`.
