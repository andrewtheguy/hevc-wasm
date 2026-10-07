# What remains of the pure-Rust decoder

What the decoder does today is in [the architecture](architecture.md), with
where its time goes; how it is built and what it measures against the
release before are in the [README](../README.md). This is the list of what it does
not do yet, in the order the work would go. Numbers are from the module
under Node on one thread of the x86 workstation, profiled with `perf record`
of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Speed

### The capture

1. **The sub-block walk.** `sub_blocks` at 14% is 154 K sub-blocks a
   picture, 127 K of them not coded: a coded-sub-block flag each, at about
   75 instructions a sub-block of which the flag's decode is near half. The
   rest is the scan table, the neighbour flags and what V8 spills around the
   call to `sub_block` for the coded ones, the range among them, reloaded
   on the bin's chain. Three shapes measured no faster: an inner loop over
   the uncoded sub-blocks alone; a function of their own with the grid
   index from the scan table and nothing live but what a flag needs, which
   keeps the range in a register and saves 1% of the instructions, 44.5 M
   cycles against 43.7 M; and that function carrying the two contexts'
   models along the run in registers, which V8 compiles to conditional
   moves, 44.2 M. The flag's chain is the range's, as the per-bin item
   says, and the walk is at its floor.

2. **The per-bin floor.** What a context-coded bin costs is a dependency
   chain, not its instruction count. The chain runs from one bin's range to
   the next's: the quartile's shift count, the LPS byte's select, the
   subtraction, and the renormalisation's four operations, about nine
   cycles, with the models' store-to-load forwarding alongside it; branch
   mispredictions are 0.11 M a picture, next to nothing. Unrolling the
   significance flags by position, testing the refill on the low word of
   the offset, the renormalisation as a shift and moving the data pointers
   out of the engine's registers cut the capture from 112 M to 106 M
   instructions and its cycles not at all; packing the four quartiles' LPS
   of a model in one word, so the lookup by model no longer waits for the
   range, cost 1% of the instructions and saved 4% of the cycles. Carrying
   the model from one bin to the next in a register where the contexts
   repeat measured nothing, and a renormalisation written as a select comes
   out of LLVM as a shift by the flag, not a `cmov`. What is left is the
   chain's shape: the renormalisation is four operations where a conditional
   move would be two, and neither LLVM nor V8 produces one here.

3. **The transform's floor.** 21%, nearly all of it the sparse path's row
   pass: a broadcast and eight dots per pair of coded columns per row, over
   32 rows, then the row's eight sums rounded, narrowed and stored. Two rows
   per pass, to load each table vector once for both, measured no faster
   (the sixteen sums spill); the column pass against a widened matrix, so
   V8 emits multiply-adds rather than its unpack-and-multiply lowering of
   `extmul`, measured the same. What is left is the shape of the loop V8
   sees, as with the bins.

### The video

4. **The row-start copy.** It was the largest item at 17%, and
   memory-bound. Two thirds of it is no longer made: the buffer reused is
   one holding a picture the first reference descends from, and a row copies
   only the runs of coding tree blocks changed since (see the architecture).
   What is left is about 10%, the copy of the other third, and that third is
   larger than it need be: a block counts as changed when any of the four
   beside it wrote a sample, whatever the strength of the edge between them,
   and when its SAO is on, whatever the offsets do. 1,400 of the video's
   2,040 blocks are left as the picture before by that rule, where nine
   tenths of its coding units are skipped; settling it from the boundary
   strengths of the block's own edges is the untried lever. The copy a
   stripe ahead of the decode rather than whole rows ahead measured slower
   at every width, one coding tree block to eight, by vector or by
   `memory.copy`: the copy is bound by the reference's lines arriving, not
   by the picture's being touched twice. The capture has almost no such
   blocks (13 of 1,600) and fifteen references between the first and a free
   buffer, so it gains nothing.

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
   video slower (2.0 ms on one, 2.4 ms on four), while the cycles rise from 11 M to 14 M.
   The video's picture is 2 ms of work spread over its few coding tree
   block rows, and the wavefront's waits (`run_rows` 2.2%, `wait_for` spinning 256
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
