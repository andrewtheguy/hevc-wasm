# What remains of the pure-Rust decoder

What the decoder does today is in [the architecture](architecture.md), with
where its time goes; how it is built and what it measures against the
release before are in the [README](../README.md). This is the list of what it does
not do yet, in the order the work would go. Numbers are from the module
under Node on one thread of the x86 workstation, profiled with `perf record`
of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Speed

### The capture

Nothing untried is left on the capture's three hot spots, the sub-block walk
(14%), the context-coded bin and the transform's row pass (21%): each is at
the shape of the loop V8 compiles, the bin's cost being the dependency chain
from one bin's range to the next's. Measured and no faster, so not to be
tried again as they were: for the walk, an inner loop or a function of their
own for the uncoded sub-blocks, with or without the two contexts' models in
registers; for the bin, carrying the model from one bin to the next where
the contexts repeat, and a renormalisation written as a select, which LLVM
turns into a shift by the flag and not a conditional move; for the
transform, two rows per pass and a widened matrix for multiply-adds.

### The video

1. **The row-start copy**, about 10% with the wavefront's waits. A third of
   the coding tree blocks are still copied: 18% of them because a coding
   unit wrote samples, 10% because the deblocking of an edge reached into
   them and nothing else did, 6% for their SAO. The deblocked ones are
   copied whole for the three samples along one side the filter can change;
   copying those lines or columns alone is the untried lever. The copy a
   stripe ahead of the decode rather than whole rows ahead measured slower
   at every width, one coding tree block to eight, by vector or by
   `memory.copy`: the copy is bound by the reference's lines arriving, not
   by the picture's being touched twice. The capture has almost no still
   blocks (13 of 1,600) and fifteen references between the first and a free
   buffer, so it gains nothing.

2. **`coding_quadtree`**, 10.3%, is a 20 KB function with no hot spot: the
   recursion, the skip and merge flags, the map fills. Moving the coding unit
   out into a function of its own makes no measurable difference. What is
   left is the fills: the `memory.fill` calls into V8 (`memory_fill_wrapper`)
   are not free even at the word size, and the 4×4-granular maps (prediction
   mode, motion, QP, transform depth) could shrink to the 8×8 the smallest
   coding unit has where nothing reads them finer.

3. **Intra prediction's reference gathering**, 9.6%: about 950 cycles a call
   for 1,166 calls a picture, most of it collecting the neighbouring samples
   with an availability check per four samples and a byte-wise substitution of
   the unavailable ones. The availability of the left column and top row is a
   per-block mask that is computable once; the gather and the substitution
   then run by vector.

4. **The in-loop filters**, about 20% together (`edge_strengths` 7.4%, the
   deblocking 7.5%, SAO 2.9%, `filter_after` 2.1%). A block with no strength
   is no longer scanned, so the deblocking's share is the 4,600 edges a
   picture it filters, and the strengths' is their computation per coding
   and transform unit, a skipped unit among skipped ones included. Neither
   is profiled to the instruction yet.

5. **`prediction_unit` and `merge_motion`**, 9.1% together: for 3,880
   prediction units, mostly merge index zero on a still block. The parsing is
   a few bins; the cost is the per-unit motion fill and the availability
   lookups for the first candidate. Not profiled to the instruction yet.

6. **Four-thread scaling.** On the capture four threads are 2.6× one; on the
   video slower (1.9 ms on one, 2.3 ms on four), while the cycles rise from 11 M to 13 M.
   The video's picture is 2 ms of work spread over its few coding tree
   block rows, and the wavefront's waits (`wait_for` spinning 256
   times before it sleeps) are a visible share. Whether the loss is the waits,
   the row-start copies contending for memory bandwidth, or rayon's scope per
   picture is not measured; `perf stat` on the pool threads separately would
   say.

### Elsewhere

- **The target machine.** All of the README's numbers are from an x86
  workstation under Bun. The module has not
  been run on Apple silicon, where V8 lowers SIMD128 to NEON differently and
  the memory system differs; the capture's transform and the video's
  memory-bound copy may rank differently there.
- **A display in strips is not benchmarked as the page decodes it.**
  `bench/run.sh` hands every unit to `decode`, a strip as if it were a whole
  picture, where remotex's page calls `decodeStrip` and reads the display at a
  frame's last strip. So the placing of each strip in the display and the
  reading of it are not in any measurement of the `strips-*` samples, which
  are of decoding alone, and a picture there is a strip, up to four to a
  frame. What it wants: each unit's strip number beside a sample, read from
  the references its slice header names as remotex's
  `tests/hp_strips_video.py` reads them; `bench/decode.ts` decoding by
  `decodeStrip` with it and reading the display once a frame; and the check
  comparing each strip as placed with FFmpeg's digest of it. FFmpeg has no
  such step, so the comparison it gives is the module in strips against the
  module decoding alone.
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

- **Fixtures.** `bun test` decodes the three `mac-*` fixtures (330×194 and
  352×256) on one thread and four; the real captures and the video are
  checked by hand with `hevc-bench` and the Node bench under `tmp/`. A
  fixture nearer the Mac's size, and a size change at an IDR mid-stream,
  belong in `test/`.
- **Strips.** The fixtures taken as strips are x265's, whose pictures refer
  to each other as a whole picture's do. The Mac's own four-strip streams,
  with their keyframe of one IDR and three intra pictures, are checked by
  hand: captures at 1280×800, 1440×900 and 1920×1080 decode bit for bit as
  FFmpeg decodes them, and come out of the module as the display.
