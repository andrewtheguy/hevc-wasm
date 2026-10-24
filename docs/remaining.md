# What remains of the pure-Rust decoder

What the decoder does today is in [the architecture](architecture.md), with
where its time goes; how it is built is in the [README](../README.md), and
what it measures against FFmpeg and the release before in
[the benchmarks](benchmarks.md). This is the list of what it does not do
yet, in the order the work would go. Numbers are from the module
under Node on one thread of the x86 workstation, profiled with `perf record`
of V8's JIT output (`tmp/jitprof.sh`), unless stated otherwise.

## Speed

The streams are the Mac's own, taken on 2026-10-12: seven scenes of a
1440×900 display in four strips, at 1x and at 2x, a picture being a strip.
Per picture on one thread, the module as it is against native FFmpeg,
under V8 on the idle VM (`bench/run.sh`, `perf stat`):

| | 1x, M cycles | FFmpeg | 2x, M cycles | FFmpeg |
|---|---|---|---|---|
| A still page | 1.1 | 1.9 | 2.1 | 7.5 |
| A window dragged | 1.4 | 2.1 | 4.2 | 8.3 |
| A quarter of the display in a loop | 1.8 | 2.4 | 5.1 | 9.5 |
| Typing in a terminal | 1.5 | 2.5 | 3.5 | 9.0 |
| Chrome scrolling an article | 3.4 | 4.1 | 9.6 | 13.1 |
| The same four times as fast | 6.9 | 7.3 | 17.3 | 20.4 |
| A video over the whole display | 15.3 | 14.3 | 28.0 | 28.3 |

The module is ahead wherever most of a picture is the picture before, and on
the scrolls; it is behind only where every picture is new, on the video at
1x by 7%, and a digit ahead on it at 2x. Those are also the streams that cost
anything. The video at 2x is 7.2 ms a strip on one thread, 29 ms for a
frame's four where 30 frames a second leave 33 ms, and 3.1 ms a strip on
four threads.

In the order the work would go:

1. **The earlier decisions, measured again.** Every step kept before
   2026-10-12 was kept on `cap5`, a 1600×1000 capture in which 140 of 236
   pictures decode to the picture before them, or on an x265 video, and
   every experiment dropped was dropped on them. The module as each kept
   step left it, built from that step's commit, has now been measured on the
   fourteen captures
   ([the benchmarks](benchmarks.md#the-steps-that-got-here)), and all but
   three earn their place there:

   - **The transform of a block with few coefficients over its coded columns
     alone**, a fifth of `cap5`'s instructions: on the captures under Bun no
     stream but the typing at 2x took fewer cycles for it as it was made,
     3%, and the video 2% to 4% more; under V8 on the idle VM it is 9%
     fewer on the typing at 2x and 1% to 2% fewer on the video. Measured since in the module as it is,
     where the coefficient list feeds it: every block sent to the dense
     transform is 8% more cycles on the typing at 2x, 2% on the video and 1%
     on the scrolls, and none of three narrower rules for which blocks take
     it (32×32 alone, a quarter of the coefficients, both) is ahead of the
     rule there is on any stream. It stays.
   - **The walk over a block's sub-blocks in a function of its own**: level
     on every stream as it was made, 1% more on the video. Inlined again in
     the module as it is, it is level still, and stays. What was not level
     is the function beside it, a coded sub-block's: decoded in the walk's
     function and not one of its own it is 2% fewer cycles on the video at
     1x, the fast scroll at 1x and the scroll, 1% on the video and the fast
     scroll at 2x, and level where little is coded. That is done, and is not
     in the table above, which is of the night before.
   - **A row copying only the blocks changed since its buffer's picture**:
     up to 5% more on a light strip under Bun and level under V8, since no
     strip ever found such a buffer. It pays since item 3 keeps the buffer, 35% of a still strip's
     cycles at 2x and 20% of the typing's, and nothing is to be done.

   Then the five experiments dropped on the old streams, listed at the end
   of this section with what each came to. None of their code was kept, so
   each is written again on the module as it is and measured on the
   captures. All five are done and no faster there either: the
   context-coded bin's two, the sub-block walk's models in registers, the
   coding unit out of `coding_quadtree`, the transform's two, and the
   row-start copy a stripe ahead. The three dropped since,
   the first pass read ahead and its taps added in pairs (item 2) and one
   thread for a small picture (item 4), were dropped on the captures and
   stand.

2. **What the profile of the video and of the fast scroll at 2x ranks.**
   Release 0.0.8 on one thread under Node, `perf record` with V8's JIT
   symbols, as shares of the samples:

   | | Video | Fast scroll |
   |---|---|---|
   | `prediction_unit` | 23.9% | 8.6% |
   | `residual::sub_block` | 15.5% | 18.8% |
   | `residual_block` | 14.2% | 8.0% |
   | `SaoCtx::component` | 11.6% | 6.3% |
   | `DeblockCtx::ctb` | 6.0% | 9.7% |
   | `intra_predict` | 4.7% | 17.6% |
   | `coding_quadtree` | 3.2% | 5.3% |
   | libc under V8, one address | 2.6% | 4.2% |
   | `residual::sub_blocks` | 2.2% | 2.1% |
   | `edge_strengths` | 2.0% | 2.7% |
   | `transform_tree` | 1.3% | 1.6% |
   | `simd128::dense_transform` | 1.3% | 1.9% |

   The residual's parsing is a third of either stream (`sub_block`,
   `residual_block`, `sub_blocks`, `coeff_remaining`: 33% and 30%), and the
   transforms next to nothing. Past that the two differ. The video is inter
   prediction, a quarter of it in `prediction_unit` with whatever of the
   interpolation is inlined there, then SAO. The scroll is intra prediction,
   18%, and the deblocking. Every ranking before this one was of
   `cap5`, a 1600×1000 capture in which 140 of 236 pictures decode to the
   picture before them at 37 KB each, or of an x265 video. Release 0.0.8, 8%
   fewer cycles than 0.0.7 on `cap5`, is 3% fewer on the video and 2% or less
   on every other stream.

   To the instruction, with a build that keeps the inlined parts as
   functions to name them (its totals are the release's, 24.4% for 24.0% and
   16.8% for 17.7%):

   | `prediction_unit` on the video | | `intra_predict` on the fast scroll | |
   |---|---|---|---|
   | `fir_hv_uni::<8>`, luma's second pass | 5.2% | `angular_t`, the modes along the left column | 4.5% |
   | `fir_hv_uni::<4>`, chroma's | 5.0% | the availability and the gathering | 4.4% |
   | `fir_h::<4>`, chroma's first pass | 3.5% | `substitute` | 2.7% |
   | `fir_h::<8>`, luma's | 3.2% | `predict`: the mode, DC, the projected reference | 1.9% |
   | The one-pass filters, both sizes | 3.5% | `angular` | 1.3% |
   | The whole-sample copy | 1.2% | `planar` | 1.1% |
   | Parsing, `amvp`, `mvd_coding`, `pb_avail` | 2.8% | `filter` | 1.0% |
   | `motion_compensate`, the padding | 1.3% | | |

   The filters are 20 of `prediction_unit`'s 24 points, and the chroma, two
   whole planes under 4:4:4, costs what the luma does.

   Three of these were done in release 0.0.9, every picture of every stream
   still FFmpeg's. In M cycles a picture on one thread, as measured then and
   before item 3 was done (the table above is the module as it is):

   | | 0.0.8 | 0.0.9 | FFmpeg |
   |---|---|---|---|
   | The video at 2x | 41.8 | 39.2 | 34.2 |
   | The fast scroll at 2x | 25.4 | 23.3 | 24.0 |
   | The fast scroll at 1x | 10.1 | 9.4 | 8.8 |
   | Typing at 1x | 2.3 | 2.3 | 3.0 |

   - **The second pass of an interpolation** was 10% of the video, each tap
     widened to 32 bits and multiplied by `vpmulld` once for each half. It is
     `i32x4_dot_i16x8`, `pmaddwd`, over two rows interleaved: 4% fewer cycles
     on the video at 2x, 2% at 1x.
   - **`angular_t`'s turn** was 3.8% of the fast scroll, a byte at a time with
     a bounds check each and a division a row. It is turned by vector, and
     the temporary is not cleared: 3.5% to 4% fewer on the fast scroll.
   - **The gathering** kept a flag a sample and looked the decoding order up
     for every four. What is there of a side is a run from the block's
     corner, so it is two lengths and one lookup a side: 4.5% fewer on the
     fast scroll, 2% on the video.

   What is left of `prediction_unit` is the first pass, 6.7% of the video,
   eight loads and eight `vpmullw` for eight luma samples, and two things
   tried on it were no faster (the video at 2x and at 1x, five rounds):

   - **The reference's rows read ahead**, a sample of every cache line of all
     three planes' footprints before the first is filtered, since the first
     load of a row holds 40% to 50% of the loop's samples: 2.8% and 1.9% more
     cycles, the cache misses as many as before.
   - **A symmetric filter's samples added in pairs** before the multiply,
     which the half-sample filters allow: 39.4 M for 39.3 M and 21.0 M for
     21.0 M.

   Untried there: leaving out the tap that is zero in luma's quarter-sample
   filters, an eighth of a 3% loop at most.

   The libc share is neither function's: by call graph all of it is under the
   wavefront's row loop, the row-start copy of item 3.

3. **The reference copied whole for every strip**, done. Each row starts as
   its reference's, and the copy is spared where a free buffer holds a
   picture the reference descends from. With a display in strips that never
   happened: a strip's reference is the same strip of the frame before, and
   the buffer free by then was another strip's, which the next picture took
   for want of a better. A free buffer that a reference descends from is now
   left for the picture decoded from that reference, and a new one is made
   when no other is free; the pool is four buffers as before. Of a strip's
   1,350 coding tree blocks at 2x, 126 are now copied a picture on the still
   page, 299 on the typing, 556 on the loop, 1,067 on the video and some
   1,210 on the scrolls. On one thread that is 35% fewer cycles on the still
   page at 2x and 17% at 1x, 20% and 9% on the typing, 7% on the loop, and
   level on the drag; the scrolls and the video take 1% more, their changed
   blocks copied by the run where a whole row went in one `memmove`. On four
   threads a light strip at 1x is no quicker by the clock for it (item 4).

   What is copied is what the reference changed, whether or not this
   picture changes it again: of the video's blocks 896 have a coding unit
   that wrote samples, 848 an edge the deblocking reached and 841 SAO, so
   copying only the lines a filter can change is worth little there.

   The copy is the largest share of a light stream at 2x in release 0.0.12,
   the row's copy and the copy kernel together: 31% of the window dragged,
   20% of the loop, 15% of the still page, before whatever of V8's own
   time is the `memmove`. Tried on that and not kept: no copy at the
   start of a row, and a block that is the first reference's where it was
   copied only where the buffer does not hold it, so that a block the
   picture predicts otherwise is never copied. Three rounds, one thread:
   level on the drag, the loop and the still page, 5.8 M cycles for 5.7 M,
   6.9 M and 2.7 M either way; 3% more on the scrolls, 13.2 M for 12.8 M
   and 23.6 M for 23.0 M, and 4% on the typing at 2x; 1% fewer on the video
   at 2x, 37.9 M for 38.4 M. On four threads the still page and the typing
   at 2x were a median 0.55 ms and 0.53 ms for 0.58 ms and 0.60 ms and the
   rest level. What that suggests, unmeasured, is that the copy's time is the
   reference's lines arriving, which the prediction of the same block then
   finds in the cache, so that sparing the copy moves the wait to the
   prediction.

4. **Threads on a small picture**, done. What a light strip paid for its
   threads was their waking, and less than the tables said: a still strip
   at 1x is 0.11 ms on one thread and was 0.15 ms on four, the typing 0.12
   and 0.17, where under `perf stat`, whose counting lengthens each waking,
   they were 0.29 and 0.34, which is what every table before 2026-10-24
   showed, three times the gap there is. The thread that asks for the
   picture has been one of the four since `8180421`, with the first row
   under way while the pool's wake, 0.16 ms for 0.19 on the still page.

   Every picture of fourteen streams decoded by a one-thread decoder and a
   four-thread one side by side (`tmp/threads-choice`, the two in turns,
   the order swapped every unit, and once more with 16 ms between units as
   a page has between frames): at 1x, under 300 bytes of slice data, one
   thread is the quicker for four pictures in five, by some 0.05 ms, a
   third of the strip; above 400 to 600 bytes four threads are. At 2x the
   bytes say little of the time: a still strip under 300 bytes is 0.34 ms
   on one thread and 0.36 on four, and a window dragged, of the same bytes,
   0.84 ms and 0.57, its cost the motion compensation over the strip, which
   its bytes do not carry; no threshold in bytes beat four threads there,
   as the earlier try of one had found, and over the fourteen streams one
   of 300 bytes alone was behind four threads for every picture, 33.4 s
   for 33.3. So a picture
   under 300 bytes and 2^19 luma samples, a light strip of a display at 1x,
   decodes on the calling thread alone and the pool sleeps (`ALONE`,
   `ALONE_SAMPLES` in `decoder.rs`); any other wakes it. On four threads a
   still strip at 1x is now 0.11 ms for 0.15 and the typing 0.14 ms for
   0.17, the drag, the loop and the scroll at 1x level, and every stream at
   2x level; paced at 16 ms, a still strip at 1x is 0.15 ms for 0.21.

   The two constants are the lab's: the waking is the kernel's, and the
   crossover in bytes moves with the machine, lower on a slower one. What
   is safe to count on is the sign at 1x, where the lightest strip takes
   less than a waking. Not kept, before this: the picture closed to a
   thread that wakes after the rows are done, so that the caller returns
   without it, which left all ten streams measured level, so the cost is
   the waking itself and not the wait for it. A rule from what the decoder
   learns as it decodes, the picture before's motion, say, would carry the
   choice to 2x, where a still page would gain 0.02 ms, not worth its
   counter.

5. **SAO's edge offset**, done for what it was worth. `SaoCtx::component`
   was 12.8% of the video at 2x, two thirds of it the edge offset's loop,
   which kept its constants in memory and loaded them again for every
   sixteen samples. A row of it is now a function of its own, whose
   constants stay in registers, and an offset is added as a signed
   saturating add of bytes where the samples had been widened to 16 bits and
   narrowed again: 3.4% fewer instructions on the video at 2x and 2% at 1x,
   1% to 6% on the scrolls and the loop, and in cycles 39.2 M for 39.9 M on
   the video at 2x, 20.3 M for 20.4 M at 1x, and 0.1 to 0.2 M on the scrolls
   and the loop at 2x. SAO is 10.9% of the video after it. What is left of
   `component` is the copy of a block and its borders that the edge offset
   reads its neighbours from; copying the column to the right with the
   block, in place of a loop down it, was 1.5 M instructions a picture more.

   The band offset was the other half of it on the loop, where
   `component` was 13.8% of the stream at 2x in release 0.0.12 and some
   40% of that the band offset's loop, inlined there: its four constants
   made again for every sixteen samples, and the offset looked up in two
   tables of sixteen bands. A row of it is now a function of its own, as
   the edge offset's is, and the band is counted from the first of the
   four that have an offset, so that one table holds them: 6.5 M cycles for
   6.7 M on the loop at 2x and 2.3 M for 2.4 M at 1x, one thread, five
   rounds, 1% fewer on the video and the scrolls at 2x, 37.8 M for 38.1 M,
   12.4 M for 12.5 M and 22.4 M for 22.6 M, and level on the drag and the
   typing.

6. **What the earlier profiles named**, on the x265 video, each to be looked
   at again only if the profile of item 2 ranks it:
   - **`coding_quadtree`'s fills**: the `memory.fill` calls into V8 are not
     free even at the word size, and the 4×4-granular maps (prediction mode,
     motion, QP, transform depth) could shrink to the 8×8 the smallest coding
     unit has where nothing reads them finer.
   - **The in-loop filters** (`edge_strengths`, the deblocking, SAO,
     `filter_after`) and **`prediction_unit` with `merge_motion`**, neither
     profiled to the instruction. The deblocking has been since, in the
     module after release 0.0.11: 11.8% of the fast scroll at 2x and 7.7%
     of the video, of which the luma filter is 5.3 and 3.7 points, the
     chroma's 2.3 and 0.9, the thresholds of an edge 2.4 and 1.6 and the
     scan of the strengths 1.8 and 1.5, and flat within each. A filter took
     four lines of an edge in vectors that hold eight; it takes eight, the
     two segments an edge on the 8×8 grid always comes in, each with its own
     thresholds and decision: 1% fewer cycles on the fast scroll and the
     video at 2x on one thread, 22.8 M for 23.0 M and 38.1 M for 38.5 M,
     five rounds, and level at 1x and on the light streams. That is less
     than half the filter's share, and what is hot in it now is the first
     load of each line of a plane, whose lines at 2x are 2,880 bytes apart
     in three planes; whether those are misses of the cache or of the page
     table has not been measured.
   - **`residual_block`**, 15.8% of the video at 2x after the eight-line
     deblocking, is little of the parsing its name suggests, that being
     `sub_blocks`, 18.4% and flat: it is the transform of a block with few
     coefficients, whose second pass is 17% of the function, and the
     residual added to the picture, 16% of it, eight samples at a time. A
     row of sixteen or more is now added sixteen at a time: 37.9 M cycles
     for 38.2 M on the video at 2x and 12.7 M for 12.8 M on the scroll, one
     thread, five rounds, and level on the video at 1x, the fast scroll and
     the typing. On the typing at 2x the largest shares after `sub_blocks`
     and `residual_block` are the copy of the blocks the reference changed,
     10% and waiting on the reference's lines, `coding_quadtree`, 9% and
     flat, and `edge_strengths`, 7.5%.

Measured on `cap5` or the x265 video and no faster, which ruled none of
them out on the captures, so each is tried again there (item 1), written
again from its description here, the code not having been kept. None is
faster there either, on one thread unless said, against the module as it
is:

- **The sub-block walk:** an inner loop or a function of their own for the
  uncoded sub-blocks, with or without the two contexts' models in registers.
  On the captures, the two models held in registers through the walk and
  stored after it: level on all nine streams measured, the video at 2x
  39.1 M cycles for 39.1 M and the typing at 2x 4.7 M for 4.6 M.
- **The context-coded bin:** carrying the model from one bin to the next
  where the contexts repeat, and a renormalisation written as a select, which
  LLVM turns into a shift by the flag and not a conditional move. On the
  captures, the model of a sub-block's greater-than-1 flags carried while
  their context repeats: level, the video at 2x 39.2 M cycles for 38.9 M.
  And the probable symbol's renormalisation as a branch with the refill
  inside it, the shift as it is being the select's: fewer instructions and
  5% to 9% more cycles on the video and the scrolls, 40.8 M for 38.9 M on
  the video at 2x, the branch being one the processor cannot guess.
- **The transform:** two rows per pass, and a widened matrix for
  multiply-adds, both of a block with few coefficients. On the captures,
  its second pass two rows at a time, each table vector loaded once for
  both, and its column pass against the matrix in 32-bit lanes, one
  multiply-add where there are two `extmul`s, each alone and the two
  together: level on all nine streams measured, the video at 2x 38.7 M,
  38.4 M and 38.4 M cycles for 38.5 M and the typing at 2x 4.6 M all four.
- **The row-start copy a stripe ahead of the decode** rather than whole rows
  ahead, at every width from one coding tree block to eight, by vector or by
  `memory.copy`: the copy is bound by the reference's lines arriving, not by
  the picture's being touched twice. Tried again on the captures in one
  form only, each block copied by the vector loop just before it is decoded
  or two or six blocks before: 23% more instructions and 22% more cycles on
  the scroll at 2x on one thread, the same at every distance, which is the
  cost of the vector loop where a row with no block to keep goes in one
  `memory.copy`, and said nothing of copying ahead. Then as it should be,
  the runs of blocks to copy taken a stripe at a time before the stripe's
  first block is decoded, a line of 128 samples or more by `memory.copy`,
  three rounds: four blocks ahead is 5% to 7% more cycles on the scrolls
  at 2x on one thread, 13.6 M for 12.7 M and 23.8 M for 22.7 M, and 1% on
  the video; sixteen ahead is level to 1% more, 12.8 M, 22.9 M and 38.3 M
  for 38.1 M. On four threads, where the copy as it is falls in a row's
  wait on the row above, neither is ahead by the clock: the scroll at 2x a
  median 1.90 ms and 1.93 ms for 1.88 ms, the video 4.42 ms and 4.36 ms
  for 4.44 ms, the typing 0.65 ms and 0.72 ms for 0.68 ms.
- **The coding unit moved out of `coding_quadtree`** into a function of its
  own. On the captures: level on all nine streams measured, a still strip
  at 2x 2.9 M cycles either way and the typing 4.7 M for 4.6 M. Forced
  inline it is the module as it is, byte for byte.

### Elsewhere

- **The target machine.** All of [the benchmarks](benchmarks.md) are from an x86
  VM, four cores of an AMD EPYC Rome, under Node's V8. The module has not
  been run on Apple silicon, where V8 lowers SIMD128 to NEON differently and
  the memory system differs; the capture's transform and the video's
  memory-bound copy may rank differently there.
- **The native build runs scalar fallbacks**: the kernels are written for
  `core::arch::wasm32`. `hevc-bench` is for correctness, not speed. A native
  SIMD port is not a goal unless the decoder gets a native user.

## Coverage

None of the fourteen captures of 2026-10-12, the seven before them or the
x265 video trip the refusals listed in the architecture. What is not known is whether the Mac ever does: in particular
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
