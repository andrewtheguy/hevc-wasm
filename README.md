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
archive = "/path/to/hevc-wasm/dist/hevc-wasm-v0.0.7.tar.gz"
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
  place. The sample loops are SIMD128
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

It decodes every picture of the seven High Performance Mac captures tried
(2,833 pictures, 1600×1000 and 1600×256, including the long-term-refresh ones)
and of a 1080p camera video x265 encoded in the Mac's shape (2,896 pictures)
bit-identically to FFmpeg, natively on one thread and on six, and as the
WebAssembly module on one and four. The `mac-*` fixtures in `test/data` are
x265's nearest shape to the Mac's.

The comparison is of the module under Bun, as it is and as release 0.0.7,
against FFmpeg's native decoder: Debian 13's ffmpeg 7.1.5-0+deb13u1, built by
gcc 14 with `--toolchain=hardened`, which picks its vector code as it runs,
AVX2 on this i5-8500T, run as `ffmpeg -threads N -i FILE -benchmark -f null -`.
They ran on a six-core x86 workstation, alternately on the same cores, one
for one thread and four for four, and the medians of three rounds are shown.
The instruction and cycle counts are `perf stat`'s for the whole process and
do not depend on the load. The module's ms is the median of its own clock's
per picture, and FFmpeg's its process's wall time over the pictures. Per
picture, lower better:

| 1600×1000 Mac capture, 236 pictures | this decoder | release 0.0.7 | FFmpeg, native |
|---|---|---|---|
| 1 thread, ms | 11.4 | 12.7 | 14.7 |
| 1 thread, M cycles | 40.8 | 44.0 | 45.6 |
| 1 thread, M instructions | 98.2 | 107.1 | 110.3 |
| 4 threads, ms | 4.5 | 4.9 | 4.5 |
| 4 threads, M cycles | 42.0 | 44.9 | 45.6 |

| 1080p video in the Mac's shape, 2,896 pictures | this decoder | release 0.0.7 | FFmpeg, native |
|---|---|---|---|
| 1 thread, ms | 1.8 | 1.8 | 6.2 |
| 1 thread, M cycles | 11.1 | 11.1 | 18.4 |
| 1 thread, M instructions | 21.3 | 21.1 | 37.6 |
| 4 threads, ms | 2.2 | 2.2 | 2.3 |
| 4 threads, M cycles | 13.2 | 13.1 | 19.0 |

| 1440×900 Mac capture of a busy animation, 120 pictures | this decoder | release 0.0.7 | FFmpeg, native |
|---|---|---|---|
| 1 thread, ms | 27.8 | 28.1 | 35.0 |
| 1 thread, M cycles | 110.5 | 109.4 | 104.5 |
| 1 thread, M instructions | 224.3 | 219.8 | 179.4 |
| 4 threads, ms | 15.5 | 14.8 | 23.9 |
| 4 threads, M cycles | 115.5 | 115.0 | 106.6 |

On one thread the module takes 11% fewer cycles than native FFmpeg on the
capture and 40% fewer on the video, where most of a picture is the picture
before and the module neither decodes nor copies it. FFmpeg's four threads
each decode a picture of their own, where the module's decode one picture's
rows, so its four-thread time matches the module's on both, for more cycles.
The busy animation, where most of every picture changes, is the other way
about: a picture costs nearly three times the first capture's, FFmpeg takes
5% fewer cycles than the module on one thread, and the module is no faster
than release 0.0.7, by 2% more instructions. FFmpeg's ms there is a
process's wall time over only 120 pictures, start-up included.

On the Mac's dense screen content the decoder spends its time where the
stream does: CABAC residual parsing and
the 32×32 inverse transforms, of which the capture has some three thousand a
picture, each with a handful of coefficients scattered to the far corners,
which the transform runs over alone. On ordinary video, where motion compensation
dominates, what led its profile was the copy
of the first reference that each row starts as: bound by memory, the reference
and the picture being written not fitting the cache together, and now made
only for the blocks that changed. Of the steps that
got here:
running the filters inside the wavefront rather than as passes over the whole
picture was worth a fifth of the cycles on four threads, where the passes had
left the threads spinning at barriers; SAO in place rather than into a second
picture, with the interpolation filters writing samples from their last pass,
a tenth on one thread; the transform's row pass splatting each pair of
coefficients with one shuffle rather than scalar loads, a twentieth of the
capture's instructions; the transform of a block with few coefficients
running down its coded columns and across the pairs of them, rather than the
butterfly over their bounding box, a fifth of the capture's instructions and
a sixth of its cycles; the block copy running down sixteen-wide columns, a
seventh of the video's instructions; the maps filled by whole words rather
than `memory.fill` calls into the runtime, a twelfth of the video's;
shipping LLVM's output without wasm-opt, a twentieth of the capture's cycles;
and the in-loop filters, which had been a quarter of the video's cycles: the
deblocking's boundary strengths settled as the blocks decode, where one side
of each edge is known once and a skipped neighbour is one block, rather than
read back per 4×4 block at filter time, so the filter scans eight strengths
at a time and nearly all are zero, and a coding tree block with no strength
in it, five in six of the video's, is not scanned at all; the edge filter
itself by vector, its four
lines in the lanes; and SAO blocks with no offset of their own and no
neighbour's edge offset to save a line for, which on these streams is nearly
all of them, cost one check. Together a seventh of the video's cycles. And
the whole-pel copies: on these streams nearly every block is a skip with a zero
vector from the first reference, four fifths of the video's samples and all but
a hundredth of the capture's, so each row's thread starts the row as one
sequential copy of the first reference's rows, before it waits on the row
above, and such a block costs nothing; the copy by the row moves the bytes
sooner than the copies by the block did, a seventh of the video's cycles again.
And most of that copy is not made: a free buffer still holds the picture it
was decoded as, each picture knows which coding tree blocks it left as the
picture its rows started as, two thirds of the video's, and so the buffer
holding a picture the first reference descends from is that reference already
but for the blocks changed since, the only ones a row copies: a twelfth of
the video's cycles and a sixth of its time on one thread.
And the parsing, which on the capture is some six hundred thousand context
bins a picture for thirty thousand coefficients, nearly all of them the
flags of sparse 32×32 blocks: the arithmetic decoder's most probable symbol,
which most bins are, takes a short path, the range shrinking by the other
symbol's share and doubling at most to put it back; the bits read ahead carry
their own end marker instead of a count, and the slice data is padded so a
refill is one whole word with no call, so that V8 keeps the registers in
registers, which it does not for a value live across a call; the walk over a
block's sub-blocks, most of them not coded, and each coded sub-block decode
in small functions of their own, since V8 spills what a large one holds, the
sub-block's significance contexts coming from a table built at compile
time and its flags decoded unrolled by position; the merge list is built
only as far as the index, which is nearly always zero; the neighbours to
the left and above, decoded before the block wherever they are, are not
looked up in the z-order map; and a block's motion is one word, written
once per 4×4. A sixth of the capture's cycles and an eighth of the video's.
And since what a bin costs is the chain from its range to the next bin's,
not its instructions, the LPS of a model's four range quartiles packed in
one word, so that the lookup by model starts before the range is known and
the range selects a byte by a shift: 4% of the capture's cycles, where
cutting a twentieth of its instructions had saved none.
And the coefficients listed as they are parsed, each with its place: the
transform of a block with few chains them by their columns and runs down
each chain, where it had looked through every coded row of every coded
column of a block the parser wrote and it then cleared, and a coefficient's
remaining level, which few have, is read in a function apart from the
sub-block's. 8% of the capture's cycles on one thread, 5% by the list alone.
Natively the decoder runs its scalar fallbacks, since the kernels are written
for wasm32. How it is put together, and where its time goes, is in
[docs/architecture.md](docs/architecture.md); what it does not do yet, in
[docs/remaining.md](docs/remaining.md).

The routine measurement of a change is `bench/run.sh [BUILD...]`, some
minutes: each BUILD is a directory holding a build of the module, `build/out`
when none is named, or the word `ffmpeg` for the host's native decoder. It
decodes the three streams above, `cap5`, `sample` and `busy-1440x900`, which
`bench/samples.sh` copies once from the `bench` folder of the artifacts drive
to `tmp/bench`, and no run reads the drive; `SAMPLES` names others of that
folder to decode instead, and `PICTURES` cuts each short. The script does
what a busy host needs: it checks the first build against FFmpeg's MD5s,
pins the builds to the same cores (`taskset`), alternates them round by
round, waits for the load to fall before each stream, and prints
`perf stat -e instructions:u,cycles:u` beside the times, with the medians of
three rounds; wall-time medians drift with the load, the counts do not.

## Requirements

The page must be cross-origin isolated for `SharedArrayBuffer`, the threads'
shared memory: remotex's gateway serves every file with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` when `[hevc_wasm]` is enabled. The
browser must run shared-memory SIMD WebAssembly and build an I444 `VideoFrame`.
