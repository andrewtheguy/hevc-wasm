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
Per picture on one thread, release 0.0.10 against native FFmpeg
(`bench/run.sh`, `perf stat`):

| | 1x, M cycles | FFmpeg | 2x, M cycles | FFmpeg |
|---|---|---|---|---|
| A still page | 1.5 | 2.4 | 2.8 | 9.0 |
| A window dragged | 2.0 | 2.6 | 5.8 | 9.7 |
| A quarter of the display in a loop | 2.5 | 3.2 | 7.0 | 11.2 |
| Typing in a terminal | 2.1 | 3.0 | 4.7 | 10.7 |
| Chrome scrolling an article | 4.8 | 4.9 | 13.2 | 15.6 |
| The same four times as fast | 9.4 | 8.8 | 23.5 | 24.0 |
| A video over the whole display | 20.3 | 17.0 | 39.2 | 34.0 |

The module is ahead wherever most of a picture is the picture before, and on
the scroll; it is behind where a picture is new, by 19% and 15% on the video
and by 7% on the fast scroll at 1x. Those are also the streams that cost
anything. The video at 2x is 12 ms a strip on one thread, 48 ms for a
frame's four where 30 frames a second leave 33 ms, and 4.5 ms a strip on
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
     alone**, a fifth of `cap5`'s instructions: on the captures no stream
     but the typing at 2x took fewer cycles for it as it was made, 3%, and
     the video took 2% to 4% more. Measured since in the module as it is,
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
     up to 5% more on a light strip, since no strip ever found such a
     buffer. It pays since item 3 keeps the buffer, 35% of a still strip's
     cycles at 2x and 20% of the typing's, and nothing is to be done.

   Then the five experiments dropped on the old streams, listed at the end
   of this section with what each came to. None of their code was kept, so
   each is written again on the module as it is and measured on the
   captures. Four are done and no faster there either: the context-coded
   bin's two, the sub-block walk's models in registers, the coding unit
   out of `coding_quadtree`, and the transform's two. One is left: the
   row-start copy a stripe ahead, tried in a form that did not test it and
   put off, a round of it being half an hour of the machine. The three dropped since,
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
   before item 3 was done (the table above is release 0.0.10):

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

4. **Threads on a small picture.** Four threads cost more than one wherever a
   strip is light, and less than they did: the thread that asks for the
   picture is now one of the four, with the first row under way while the
   pool's wake, where it had slept until the pool was done. Every strip is
   0.1 to 0.2 ms sooner for it on four threads: a still page at 1x 0.29 ms
   for 0.49 ms, the typing 0.39 ms for 0.61 ms, the video at 2x 4.55 ms for
   4.66 ms. One thread still does a still strip at 1x in 0.17 ms, since the
   picture is not done until the pool's threads have woken to find nothing
   left. Threads pay on the heavy streams: 2.7× on the video at 2x, 2.2× at
   1x, 2.1× on the fast scroll at 2x, 1.6× on the scroll.

   Tried and not kept: one thread for a picture under some size in bytes.
   The size says little of the time: of the strips under 200 bytes at 2x,
   whose median is 0.61 ms on one thread and 0.67 ms on four, four threads
   were the quicker for 63%. No threshold beat four threads for every
   picture at 2x, and the best at 1x, 400 bytes, saved 4% of the time of six
   streams, by a constant fitted to this machine. What would do it is the
   pool's threads woken only once a picture is known to be worth them, by
   something the decoder learns as it decodes; it has no clock in the page.

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

6. **What the earlier profiles named**, on the x265 video, each to be looked
   at again only if the profile of item 2 ranks it:
   - **`coding_quadtree`'s fills**: the `memory.fill` calls into V8 are not
     free even at the word size, and the 4×4-granular maps (prediction mode,
     motion, QP, transform depth) could shrink to the 8×8 the smallest coding
     unit has where nothing reads them finer.
   - **The in-loop filters** (`edge_strengths`, the deblocking, SAO,
     `filter_after`) and **`prediction_unit` with `merge_motion`**, neither
     profiled to the instruction.

Measured on `cap5` or the x265 video and no faster, which ruled none of
them out on the captures, so each is tried again there (item 1), written
again from its description here, the code not having been kept. Four are
no faster there either, one thread, against the module as it is:

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
  `memory.copy`, and says nothing of copying ahead. To be tried again later
  as it should be, several blocks' width at a time by `memory.copy`, on one
  thread and on four, where the copy as it is falls in a row's wait on the
  row above; a round of it is half an hour of the machine.
- **The coding unit moved out of `coding_quadtree`** into a function of its
  own. On the captures: level on all nine streams measured, a still strip
  at 2x 2.9 M cycles either way and the typing 4.7 M for 4.6 M. Forced
  inline it is the module as it is, byte for byte.

### Elsewhere

- **The target machine.** All of [the benchmarks](benchmarks.md) are from an x86
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
