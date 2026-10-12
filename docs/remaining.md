# What remains of the pure-Rust decoder

What the decoder does today is in [the architecture](architecture.md), with
where its time goes; how it is built and what it measures against the
release before are in the [README](../README.md). This is the list of what it does
not do yet, in the order the work would go. Numbers are from the module
under Node on one thread of the x86 workstation, profiled with `perf record`
of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Speed

The streams are the Mac's own, taken on 2026-10-12: seven scenes of a
1440×900 display in four strips, at 1x and at 2x, a picture being a strip.
Per picture on one thread, release 0.0.8 against native FFmpeg
(`bench/run.sh`, `perf stat`):

| | 1x, M cycles | FFmpeg | 2x, M cycles | FFmpeg |
|---|---|---|---|---|
| A still page | 1.8 | 2.4 | 4.2 | 8.9 |
| A window dragged | 2.0 | 2.6 | 5.9 | 9.5 |
| A quarter of the display in a loop | 2.7 | 3.2 | 7.7 | 11.0 |
| Typing in a terminal | 2.3 | 3.0 | 5.9 | 10.6 |
| Chrome scrolling an article | 4.9 | 4.9 | 13.5 | 15.5 |
| The same four times as fast | 10.1 | 8.8 | 25.4 | 24.0 |
| A video over the whole display | 21.0 | 17.1 | 41.9 | 34.2 |

The module is ahead wherever most of a picture is the picture before, and
behind where a picture is new: by 23% on the video at either density, by 15%
and 6% on the fast scroll. Those are also the streams that cost anything. The
video at 2x is 12.8 ms a strip on one thread, 51 ms for a frame's four where
30 frames a second leave 33 ms, and 5.0 ms a strip on four threads.

In the order the work would go:

1. **A profile of the video and of the fast scroll at 2x**, which there is
   none of. Every ranking before this one was of `cap5`, a 1600×1000 capture
   in which 140 of 236 pictures decode to the picture before them at 37 KB
   each, or of an x265 video, and neither is what the Mac sends of a display
   that moves. Release 0.0.8, 8% fewer cycles than 0.0.7 on `cap5`, is 3%
   fewer on the video and 2% or less on every other stream. What the profile is expected to rank, and has not: the parsing
   and the transforms of a strip's 24 KB, the intra prediction of the rows a
   scroll brings in, and the in-loop filters over a picture that all changed.

2. **Threads on a small picture.** Four threads cost more than one wherever a
   strip is light: a still page at 1x is 1.8 M cycles and a 0.26 ms median on
   one thread, 2.4 M and 0.48 ms on four, and so with the drag, the loop and
   the typing, 20% to 40% more cycles. They pay only on the heavy streams:
   2.6× on the video at 2x, 2.1× at 1x, 1.6× on the scroll at 2x. Whether a
   strip under some size should be decoded on the calling thread, and by what
   the decoder would know it before decoding, is not tried. The wavefront's
   waits (`wait_for` spinning 256 times before it sleeps), the row-start
   copies and rayon's scope per picture are the three places the loss can be,
   and `perf stat` on the pool threads separately would say which.

3. **What the earlier profiles named**, on the x265 video, each to be looked
   at again only if the profile of item 1 ranks it:
   - **The row-start copy.** A third of the coding tree blocks were still
     copied: 18% because a coding unit wrote samples, 10% because the
     deblocking of an edge reached into them, 6% for their SAO. Copying the
     three lines or columns the filter can change, and not the block, is
     untried.
   - **`coding_quadtree`'s fills**: the `memory.fill` calls into V8 are not
     free even at the word size, and the 4×4-granular maps (prediction mode,
     motion, QP, transform depth) could shrink to the 8×8 the smallest coding
     unit has where nothing reads them finer.
   - **Intra prediction's reference gathering**: an availability check per
     four samples and a byte-wise substitution, where the availability of the
     left column and top row is a mask computable once per block and the
     gather could run by vector.
   - **The in-loop filters** (`edge_strengths`, the deblocking, SAO,
     `filter_after`) and **`prediction_unit` with `merge_motion`**, neither
     profiled to the instruction.

Measured on `cap5` or the x265 video and no faster, so not to be tried again
as they were:

- **The sub-block walk:** an inner loop or a function of their own for the
  uncoded sub-blocks, with or without the two contexts' models in registers.
- **The context-coded bin:** carrying the model from one bin to the next
  where the contexts repeat, and a renormalisation written as a select, which
  LLVM turns into a shift by the flag and not a conditional move.
- **The transform:** two rows per pass, and a widened matrix for
  multiply-adds.
- **The row-start copy a stripe ahead of the decode** rather than whole rows
  ahead, at every width from one coding tree block to eight, by vector or by
  `memory.copy`: the copy is bound by the reference's lines arriving, not by
  the picture's being touched twice.
- **The coding unit moved out of `coding_quadtree`** into a function of its
  own.

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
  reading of it are not in any measurement above, which are of decoding
  alone, and a picture there is a strip, up to four to a frame. The Mac does
  not send every strip of every frame: of the loop it sent about two and a
  half a frame, so a strip's number cannot be counted from its place in the
  stream. What it wants: each unit's strip number beside a sample, read from
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
