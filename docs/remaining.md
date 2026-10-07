# What remains of the pure-Rust decoder

What the decoder does today is in [the architecture](architecture.md), with
where its time goes; how it is built and what it measures against FFmpeg's
decoder are in the [README](../README.md). This is the list of what it does
not do yet, in the order the work would go. Numbers are from the module
under Node on one thread of the x86 workstation, profiled with `perf record`
of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Speed

### The capture

1. **The sub-block loop's head.** `residual_block` at 27% is not the flags it
   decodes (those are in `sub_block`) but what it does between sub-blocks: a
   coded-sub-block flag and the construction of the sub-block's description
   (its sixteen significance contexts, its greater-than-one and -two context
   sets, its position) for each of the up to 64 sub-blocks of 3,022 blocks a
   picture. Most of that description depends only on the sub-block's position
   and the block's size and component, so a table indexed by those, built once,
   replaces the arithmetic. The csbf context itself is two map lookups a
   sub-block.

2. **The per-bin floor.** The probable-symbol path of `decode` compiles to
   about 50 x86 instructions, of which the arithmetic is a dozen; the rest is
   V8 spilling and reloading the engine's state and the loop's invariants (the
   context table base, the sub-block description) around the call. 645 K bins
   at 50 instructions is over a quarter of the capture's 115 M instructions.
   The levers left are in the shape of the code V8 sees: the sixteen
   significance contexts as two 64-bit words rather than a byte array it
   reloads through a pointer, the significance loop unrolled by the scan so
   the position is a constant, and inspecting the Liftoff/TurboFan output
   (`tmp/annot.sh`) for which values spill. There is no `inline(always)`
   across the wasm boundary to help; this is the floor V8's register
   allocator sets.

3. **The transform's floor.** 21%, nearly all of it the sparse path's row
   pass: a broadcast and eight dots per pair of coded columns per row, over
   32 rows, then the row's eight sums rounded, narrowed and stored. Two rows
   per pass, to load each table vector once for both, measured no faster
   (the sixteen sums spill); the column pass against a widened matrix, so
   V8 emits multiply-adds rather than its unpack-and-multiply lowering of
   `extmul`, measured the same. What is left is the shape of the loop V8
   sees, as with the bins.

### The video

4. **The row-start copy.** The largest item at 17%, and memory-bound: each row
   begins as a copy of the first reference's rows, and the reference and the
   picture being written do not fit the cache together. The remaining lever
   is not to touch the memory twice. Candidates: copying the reference's row
   into the picture a stripe ahead of the decode rather than whole rows
   ahead, so the written lines are still cached when the blocks that are not
   still overwrite them; and reusing, as the next picture's buffer, the one
   the first reference was copied from last, so its lines are the warmest.
   Neither is measured.

5. **`coding_quadtree`**, 9.3%, is a 20 KB function with no hot spot: the
   recursion, the skip and merge flags, the map fills. Moving the coding unit
   out into a function of its own makes no measurable difference. What is
   left is the fills: the `memory.fill` calls into V8 (`memory_fill_wrapper`)
   are not free even at the word size, and the 4×4-granular maps (prediction
   mode, motion, QP, transform depth) could shrink to the 8×8 the smallest
   coding unit has where nothing reads them finer.

6. **Intra prediction's reference gathering**, 8.3%: about 950 cycles a call
   for 1,166 calls a picture, most of it collecting the neighbouring samples
   with an availability check per four samples and a byte-wise substitution of
   the unavailable ones. The availability of the left column and top row is a
   per-block mask that is computable once; the gather and the substitution
   then run by vector.

7. **The in-loop filters**, about 22% together (`edge_strengths` 6.3%, the
   edge filters 10.6%, SAO 2.9%, `filter_after` 1.8%). The boundary strengths
   are settled as blocks decode and scanned eight at a time, and the edge
   filter runs its four lines in the lanes; what remains is the visit itself:
   every edge of every block is scanned, strengths zero or not. Summarising
   the strengths per coding tree block (any non-zero at all) so that a still
   block costs one check, as SAO's blocks already do, is the untried lever.

8. **`prediction_unit` and `merge_motion`**, 8.4% together: for 3,880
   prediction units, mostly merge index zero on a still block. The parsing is
   a few bins; the cost is the per-unit motion fill and the availability
   lookups for the first candidate. Not profiled to the instruction yet.

9. **Four-thread scaling.** On the capture four threads are 2.6× one; on the
   video only 1.05× (2.3 → 2.2 ms), while the cycles rise from 12 M to 15 M.
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
  transform and the video's memory-bound copy may rank differently there.
- **The native build runs scalar fallbacks**: the kernels are written for
  `core::arch::wasm32`. `hevc-bench` is for correctness, not speed. A native
  SIMD port is not a goal unless the decoder gets a native user.

## Coverage

None of the seven captures or the x265 video trip the refusals listed in the
architecture. What is not known is whether the Mac ever does: in particular
whether it emits more than one slice when a picture is large, uses long-term
references in a shape other than the `ltr-*` captures' (which decode), or
ever sends a B picture. The refusal makes the page fall back, so a wrong
guess costs a frame, not a crash; the list is the one to revisit against a
wider set of captures (other resolutions, other macOS versions, external
displays).

## Robustness

- **Malformed input must return an error, never trap.** The fuzz target
  (`rust/hevc/fuzz`, see the README) has had forty minutes on five workers,
  about 110,000 inputs, on one thread alone, and ten minutes, about 16,000
  inputs, since it decodes each input on the pool as well and compares:
  without a panic, a sanitizer report, a timeout or a disagreement, with the
  coverage still growing when it stopped. A run of hours, and one from a
  corpus of the real captures' access units (under `tmp/`), is what would
  say more. The module's SIMD loops have only the damaged fixtures of
  `bun test` against them.
- **A failed thread.** A pool thread that does not start fails the module's
  load, and so every decoder. The pool's seats are behind mutexes that
  `expect` no thread panics while holding one; a thread that panics later
  poisons the pool for every decoder sharing the module, and the page's only
  recovery is a new decode worker, which nothing does yet. Whether the page
  can recover a wedged pool, or must reload the module, is not decided.
- **Memory.** Peak memory against the Mac's largest display has not been
  measured.

## Verification

- **Fixtures.** `bun test` decodes the two `mac-*` fixtures (330×194 and
  352×256) on one thread and four; the real captures and the video are
  checked by hand with `hevc-bench` and the Node bench under `tmp/`. A
  fixture nearer the Mac's size, and a size change at an IDR mid-stream,
  belong in `test/`.
