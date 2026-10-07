# What remains of the pure-Rust decoder

As of commit `38da479` on `pure-rust`. What the decoder does today, how it is
built and what it measures are in the [README](../README.md); this is the list
of what it does not do yet, in the order the work would go. Numbers are from
the module under Node on one thread of the x86 workstation, profiled with
`perf record` of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Where the time goes

The 1600×1000 Mac capture (`cap5`, 236 pictures), 21.6 ms a picture under the
profiler:

| share | function | what it is |
|---|---|---|
| 45.5% | `inverse_transform` | the 32×32 transforms, run dense on sparse blocks |
| 17.4% | `residual_block` | the loop over a block's 4×4 sub-blocks: coded-sub-block flags and the per-sub-block setup |
| 16.1% | `sub_block` | one sub-block's significance flags, levels and signs |
| 2.7% | libc `memmove` | the copy of the first reference each row starts as |
| 1.4% | `edge_strengths` | deblocking boundary strengths |
| 1.2% | `intra_predict` | |
| 1.2% | `coding_quadtree` | |
| 0.9% | deblock `edges` | |
| 0.8% | `memory_fill_wrapper` | `memory.fill` calls into V8 for the map fills |

The 1080p camera video in the Mac's shape (`sample`, 2,896 pictures), 4.4 ms a
picture under the profiler:

| share | function |
|---|---|
| 13.1% | `sub_block` |
| 16.8% | libc `memmove` (two symbols: the row-start copy and the stripe copies) |
| 9.3% | `coding_quadtree` |
| 8.3% | `intra_predict` |
| 6.5% | `prediction_unit` |
| 6.3% | `edge_strengths` |
| 10.6% | deblock `edges` (luma 5.6%, chroma 5.0%) |
| 4.8% | `copy_block` |
| 3.2% | `residual_block` |
| 2.9% | SAO `component` |
| 2.8% | `inverse_transform` |
| 2.2% | `run_rows` (the wavefront's waits) |
| 1.9% | `merge_motion` |
| 1.8% | `filter_after` |

Per picture of the capture, counted once with temporary counters: 4,060
residual blocks, 3,022 of them 32×32, 28.8 K coefficients in all (about seven
per 32×32 block), 645 K context-coded bins and 46 K bypass bins, so about 160
bins per 32×32 block, nearly all of them coded-sub-block and significance
flags. The video: 551 blocks, 8,244 coefficients, 45 K context bins, 19.6 K
bypass bins, 4,029 coding units of which 3,642 are skipped, 3,880 prediction
units of which 3,774 are merges and 3,202 are still blocks from the first
reference.

## Speed

### The capture

1. **A sparse inverse transform.** Nearly half the capture's time. The 32×32
   blocks carry about seven coefficients each, but scattered far enough that
   the bounded transform (`nz_w` × `nz_h` of coded rows and columns) runs over
   most of the block. A path for a block with few coefficients that
   accumulates each coefficient's outer product of basis vectors, or a column
   pass over the coded columns only followed by a dense row pass, would spend
   its time in proportion to the coefficients rather than their bounding box.
   A DC-only shortcut covers a further share of the blocks. The transform must
   stay bit-exact with the 16-bit two-stage transform it replaces: same
   intermediate clipping, same rounding per stage.

2. **The sub-block loop's head.** `residual_block` at 17% is not the flags it
   decodes (those are in `sub_block`) but what it does between sub-blocks: a
   coded-sub-block flag and the construction of the sub-block's description
   (its sixteen significance contexts, its greater-than-one and -two context
   sets, its position) for each of the up to 64 sub-blocks of 3,022 blocks a
   picture. Most of that description depends only on the sub-block's position
   and the block's size and component, so a table indexed by those, built once,
   replaces the arithmetic. The csbf context itself is two map lookups a
   sub-block.

3. **The per-bin floor.** The probable-symbol path of `decode` compiles to
   about 50 x86 instructions, of which the arithmetic is a dozen; the rest is
   V8 spilling and reloading the engine's state and the loop's invariants (the
   context table base, the sub-block description) around the call. 645 K bins
   at 50 instructions is a fifth of the capture's 146 M instructions. The
   levers left are in the shape of the code V8 sees: the sixteen significance
   contexts as two 64-bit words rather than a byte array it reloads through a
   pointer, the significance loop unrolled by the scan so the position is a
   constant, and inspecting the Liftoff/TurboFan output (`tmp/annot.sh`) for
   which values spill. There is no `inline(always)` across the wasm boundary
   to help; this is the floor V8's register allocator sets.

### The video

4. **The row-start copy.** The largest item at 17%, and memory-bound: each row
   begins as a copy of the first reference's rows, and the reference and the
   picture being written do not fit the cache together. It replaced
   per-block copies and was a seventh of the cycles cheaper, so the remaining
   lever is not to touch the memory twice. Candidates: copying the reference's
   row into the picture a stripe ahead of the decode rather than whole rows
   ahead, so the written lines are still cached when the blocks that are not
   still overwrite them; and reusing, as the next picture's buffer, the one
   the first reference was copied from last, so its lines are the warmest.
   Neither is measured.

5. **`coding_quadtree`**, 9.3%, is a 20 KB function with no hot spot: the
   recursion, the skip and merge flags, the map fills. Moving the coding unit
   out into a function of its own made no measurable difference. What is left
   is the fills: the `memory.fill` calls into V8 (`memory_fill_wrapper`) are
   not free even at the word size, and the 4×4-granular maps (prediction mode,
   motion, QP, transform depth) could shrink to the 8×8 the smallest coding
   unit has where nothing reads them finer.

6. **Intra prediction's reference gathering**, 8.3%: about 950 cycles a call
   for 1,166 calls a picture, most of it collecting the neighbouring samples
   with an availability check per four samples and a byte-wise substitution of
   the unavailable ones. The availability of the left column and top row is a
   per-block mask that is computable once; the gather and the substitution
   then run by vector.

7. **The in-loop filters**, about 22% together (`edge_strengths` 6.3%, the
   edge filters 10.6%, SAO 2.9%, `filter_after` 1.8%). The boundary strengths
   are settled as blocks decode and scanned eight at a time, and the edge
   filter runs its four lines in the lanes; what remains is the visit itself: every
   edge of every block is scanned, strengths zero or not. Summarising the
   strengths per coding tree block (any non-zero at all) so that a still block
   costs one check, as SAO's blocks already do, is the untried lever.

8. **`prediction_unit` and `merge_motion`**, 8.4% together: for 3,880
   prediction units, mostly merge index zero on a still block. The parsing is
   now a few bins; the cost is the per-unit motion fill and the availability
   lookups for the first candidate. Not profiled to the instruction yet.

9. **Four-thread scaling.** On the capture four threads are 2.75× one; on the
   video only 1.1× (2.6 → 2.4 ms), while the cycles rise from 13 M to 15 M.
   The video's picture is 2.6 ms of work spread over its few coding tree
   block rows, and the wavefront's waits (`run_rows` 2.2%, `wait_for` spinning 256
   times before it sleeps) are a visible share. Whether the loss is the waits,
   the row-start copies contending for memory bandwidth, or rayon's scope per
   picture is not measured; `perf stat` on the pool threads separately would
   say.

### Elsewhere

- **The target machine.** All of the Rust module's numbers are from an x86
  workstation under Node. The FFmpeg module's table in the README is from an
  M2 Max. The Rust module has not been run on Apple silicon, where V8 lowers
  SIMD128 to NEON differently and the memory system differs; the capture's
  dense-transform and the video's memory-bound copy may rank differently
  there.
- **The native build runs scalar fallbacks**: the kernels are written for
  `core::arch::wasm32`. `hevc-bench` is for correctness, not speed. A native
  SIMD port is not a goal unless the decoder gets a native user.

## Coverage

The decoder refuses, by name, anything outside the Mac's shape. From the
parameter sets: a chroma format other than 4:4:4, separate colour planes, a
bit depth other than 8, picture reordering, scaling lists, asymmetric motion
partitions, PCM, long-term reference pictures, strong intra smoothing, SPS and
PPS extensions, dependent slice segments, `pic_output_flag`, sign data
hiding, `cabac_init_flag`, constrained intra prediction, transform skip,
weighted prediction, transquant bypass, tiles, and a stream without the
wavefront (`entropy_coding_sync_enabled_flag` 0), reference list
modification. From the slice header: more than one slice per picture, B
slices, temporal motion vector prediction, and a slice whose entry points are
not one per coding tree block row. From the NAL header: any picture type
other than the two IDR types and the two trailing types (so no CRA, which the
Mac's stream does not use).

None of the seven captures or the x265 video trip these. What is not known is
whether the Mac ever does: in particular whether it emits more than one slice
when a picture is large, uses long-term references in a shape other than the
`ltr-*` captures' (which decode), or ever sends a B picture. The refusal
makes the page fall back, so a wrong guess costs a frame, not a crash; the
list is the one to revisit against a wider set of captures (other
resolutions, other macOS versions, external displays).

Mid-stream changes: a new SPS of another size takes a new picture buffer
(the pool is filtered by size); the tests cover joining before a keyframe and
garbage units but not a size change for the Rust module, which the FFmpeg
module's tests do cover (`test/decoder.test.ts`).

## Robustness

- **Malformed input must return an error, never trap.** A WebAssembly trap
  (a Rust panic, an out-of-bounds index, an `assert!`) is fatal to the module
  instance, and the page would have to reload it, whereas an `Err` from
  `decode` is recovered by waiting for the next keyframe. The decoder has
  about twenty `assert!`/`unwrap` sites and relies on slice indexing being
  in bounds, and the review of this round found an arithmetic wraparound in
  the new CABAC refill that could have read past the buffer on a corrupt
  stream. Nothing fuzzes it. A `cargo fuzz` target over `Decoder::decode`
  seeded with the test fixtures' access units, run natively, is the next
  step; the panics it finds are then turned into errors, and the bounds the
  CABAC engine and bit reader rely on (`end`, the RBSP padding) get tests of
  their own.
- **A panic on a pool thread.** The pool's seats are behind mutexes that
  `expect` no thread panics while holding one; a panic there poisons the pool
  for every decoder sharing it. Whether the page can recover a wedged pool, or
  must reload the module, is not decided.
- **Memory.** Picture buffers are pooled (at most four spare) and sized by the
  sequence; the input buffer grows to the largest access unit. The shared
  memory's maximum is 1 GiB (`rust/hevc-web/.cargo/config.toml`); a 2880×1800 capture
  at 4:4:4 holds 15.5 MB a picture and the DPB keeps `max_dec_pic_buffering`
  of them. Peak memory against the Mac's largest display has not been
  measured.

## Integration into remotex

The page's decode worker loads this module as of remotex's `pure-rust-hevc`
branch: the archive carries `hevc.js` and `hevc.wasm` as before, the gateway
pins release 0.0.2 by SHA-256 and serves the two at `/hevc/`, the worker
imports the glue from there and seats the pool's threads as workers of the
page's bundle (`hevcWasm.worker.ts`, `hevcPool.worker.ts`), and the paint
worker reads the planes as it did. What is left:

1. **The release.** `publish-private.sh` has not run for 0.0.2: remotex's pin
   is the digest of a local `./build.sh`, which a second build reproduced, and
   the published archive's `SHA256SUMS` must agree with it before remotex's
   branch merges.
2. **In the browser.** Both sides are checked under Bun and Node only: the
   module's tests here, remotex's unit tests there (the worker itself runs in
   no test, as the FFmpeg one did not). The Playwright spec
   `tests/playwright/software-hevc.spec.ts` needs a gateway with a live
   High Performance Mac and the archive beside its config; it asserts the files
   loaded and the first passed keyframe acknowledged without a decoder failure,
   and is the first thing to run. Nested workers (the pool's threads are
   started by the decode worker, itself a worker) are what the compositor's
   pool relies on already.
3. **A failed thread.** A pool thread that does not start fails the module's
   load, and so every decoder, as a failed worker did before; one that panics
   later poisons the pool's seats for every decoder sharing the module, and the
   page's only recovery is a new decode worker, which nothing does yet.
4. **Fixtures.** `bun test` decodes the two `mac-*` fixtures (330×194 and
   352×256) on one thread and four; the real captures and the video are checked
   by hand with `hevc-bench` and the Node bench under `tmp/`. A fixture nearer
   the Mac's size, and a size change mid-stream, which the FFmpeg module's tests
   covered and this module's do not, belong in `test/`.
5. **Two modules of one shape.** The decode worker's loader now mirrors the
   compositor's (`egfxPool.worker.ts`), differing only in where the glue comes
   from, and will go on doing so: the licence that keeps the decoder out of
   every remotex build keeps it out of the bundle, so the archive, the pin and
   `[hevc_wasm]` stay, and the two loaders stay two.
