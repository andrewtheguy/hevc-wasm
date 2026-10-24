# Benchmarks

What the decoder measures against FFmpeg and against the release before, how
a change is measured, and what each step that got it here was worth. Where a
picture's time goes inside the decoder, by function and by count, is in
[the architecture](architecture.md#where-the-time-goes); what is left to
gain, in [what remains](remaining.md).

## Against FFmpeg and the releases before

The comparison is of the module as it is, as release 0.0.10 and as release
0.0.9, under Node 26 (V8 14.6, the engine of the Chrome the module is for),
against FFmpeg's native decoder: Debian 13's ffmpeg 7.1.5-0+deb13u1, built by
gcc 14 with `--toolchain=hardened`, which picks its vector code as it runs,
AVX2 on this EPYC, run as `ffmpeg -threads N -i FILE -benchmark -f null -`.
They ran on an idle VM, four cores of an AMD EPYC Rome with two threads each
and nothing else running, alternately on the same cores, one for one thread
and four, one per core, for four, and the medians of three rounds are shown.
The instruction and cycle counts are `perf stat`'s for the whole process and
do not depend on the load. The module's ms is the median of its own clock's
per picture, and FFmpeg's its process's wall time over the pictures. The
streams are seven scenes of the display, each at 1x and at 2x, thirty seconds
of each; a picture is a strip, a quarter of the display, and the Mac sends
only the strips that changed. Strips are what zDesk asks of every such
session; one picture of the whole display is what it asks only for a display
too low for strips, under 144 rows, and for a probe, so it is not measured
here. Each ratio is the module's ms over FFmpeg's
beside it, under 1.00 where the module is the quicker; it sets a median
against a mean, so a few long pictures weigh on FFmpeg's figure and not on
the module's. On four threads FFmpeg is run with `-thread_type slice`, its
threads on the rows of one picture as the module's are, so both figures are
how long a strip takes; left to itself it decodes four pictures at once,
which is quicker over a file and no use to a display that is watched. Per
picture, lower better:

| 1440×240, a strip | pictures | 1 thread, M cycles | 0.0.10 | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,450 | 1.1 | 1.2 | 1.3 | 1.9 | 0.11 | 0.50 | $\color{green}{0.22}$ | 0.11 | 0.41 | $\color{green}{0.27}$ |
| A window dragged | 2,403 | 1.4 | 1.4 | 1.4 | 2.1 | 0.19 | 0.54 | $\color{green}{0.35}$ | 0.22 | 0.45 | $\color{green}{0.49}$ |
| A quarter of the display in a loop | 2,273 | 1.8 | 1.9 | 1.9 | 2.4 | 0.41 | 0.64 | $\color{green}{0.64}$ | 0.45 | 0.57 | $\color{green}{0.79}$ |
| Typing in a terminal | 2,248 | 1.5 | 1.5 | 1.7 | 2.5 | 0.12 | 0.64 | $\color{green}{0.19}$ | 0.14 | 0.47 | $\color{green}{0.30}$ |
| Chrome scrolling an article | 2,588 | 3.4 | 3.6 | 3.5 | 4.1 | 0.59 | 1.08 | $\color{green}{0.55}$ | 0.58 | 0.83 | $\color{green}{0.70}$ |
| The same four times as fast | 2,362 | 6.9 | 7.2 | 7.3 | 7.3 | 1.59 | 1.93 | $\color{green}{0.83}$ | 1.06 | 1.19 | $\color{green}{0.89}$ |
| A video over the whole display | 1,978 | 15.3 | 15.7 | 15.9 | 14.3 | 3.98 | 3.72 | $\color{red}{1.07}$ | 1.86 | 1.79 | $\color{red}{1.04}$ |

| 2880×464, a strip | pictures | 1 thread, M cycles | 0.0.10 | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,694 | 2.1 | 2.2 | 3.4 | 7.5 | 0.35 | 1.93 | $\color{green}{0.18}$ | 0.39 | 1.03 | $\color{green}{0.38}$ |
| A window dragged | 2,507 | 4.2 | 4.2 | 4.5 | 8.3 | 0.90 | 2.16 | $\color{green}{0.42}$ | 0.60 | 1.15 | $\color{green}{0.52}$ |
| A quarter of the display in a loop | 2,230 | 5.1 | 5.2 | 5.7 | 9.5 | 1.32 | 2.43 | $\color{green}{0.54}$ | 0.98 | 1.35 | $\color{green}{0.73}$ |
| Typing in a terminal | 2,424 | 3.5 | 3.7 | 4.5 | 9.0 | 0.32 | 2.33 | $\color{green}{0.14}$ | 0.39 | 1.20 | $\color{green}{0.33}$ |
| Chrome scrolling an article | 2,579 | 9.6 | 9.7 | 10.1 | 13.1 | 1.89 | 3.41 | $\color{green}{0.55}$ | 1.31 | 1.88 | $\color{green}{0.70}$ |
| The same four times as fast | 2,501 | 17.3 | 17.8 | 18.2 | 20.4 | 3.98 | 5.37 | $\color{green}{0.74}$ | 2.25 | 2.64 | $\color{green}{0.85}$ |
| A video over the whole display | 2,446 | 28.0 | 29.0 | 29.7 | 28.3 | 7.20 | 7.34 | $\color{green}{0.98}$ | 3.05 | 3.09 | $\color{green}{0.99}$ |

On one thread the module takes fewer cycles than native FFmpeg on every
stream but the video: two fifths fewer on the still page at 1x and nearly
three quarters at 2x, two fifths and three fifths on the typing, a quarter
to a half on the drag and the loop, 17% and 27% on the scroll, and 5% and
15% on the fast scroll, where a picture is new. The video takes 7% more at
1x and 1% fewer at 2x, 28.0 M for 28.3 M; its strip at 2x is 7.2 ms on one
thread, four of them 29 ms of the 33 ms of a frame at thirty a second, and
3.1 ms on four. Release 0.0.9 still copied the part of a picture that is
the picture before, which is the still page at 2x going from 3.4 M to
2.1 M; the four steps since 0.0.10 ([below](#the-steps-that-got-here)) are
the digit to 5% between its column and the first.

On four threads, strip for strip, the module is the quicker on every stream
but the video at 1x, where FFmpeg's rows are 4% quicker: by two thirds to
three quarters on the still page and the typing, by half on the drag, a
quarter on the loop, three tenths on the scroll and a tenth to a seventh on
the fast scroll, and level on the video at 2x. A light strip gains nothing
from threads, whose waking costs more than its few rows take: a still
strip at 1x is 0.11 ms on one thread and was 0.15 ms on four, 0.50 ms and
0.41 ms in FFmpeg, so since the last step a picture under 300 bytes at 1x
is the calling thread's alone, which is the 0.11 ms in the column, and the
typing's 0.14 ms for 0.17. All of it is one run, the afternoon of
2026-10-24, on the idle VM, whose 1-minute load as each stream's one-thread
timing ended was 0.0 to 1.5, of which the decoder being timed is 1.0, but
for the module's four-thread column, which is of that evening, after the
step, alternately with the module of the afternoon, which it is within
0.04 ms of wherever the step changes nothing. The tables before this run were of the module under Bun,
whose JavaScriptCore is Safari's, on an i5-8500T workstation that was not
idle; here the module takes a fifth to a quarter fewer cycles on every
stream and FFmpeg a sixth fewer, and which part of that is the engine's and
which the machine's the run does not say.

## Measuring a change

The routine measurement of a change is `bench/run.sh [BUILD...]`, some
minutes: each BUILD is a directory holding a build of the module, `build/out`
when none is named, the word `ffmpeg` for the host's native decoder, or
`ffmpeg-rows` for it with its threads on the rows of one picture. It
decodes four streams of a Mac's 1440×900 display in strips, `video-1x` and
`video-2x`, a video over the whole of it at 1x and at 2x, `scroll-2x`, Chrome
scrolling an article, and `typing-1x`, typing in a terminal, which
`bench/samples.sh` copies once from the `bench` folder of the artifacts drive
to `tmp/bench`, and no run reads the drive; `SAMPLES` names others of that
folder to decode instead, and `PICTURES` cuts each short. The script does
what a busy host needs: it checks the first build against FFmpeg's MD5s,
pins the builds to the same cores (`taskset`), alternates them round by
round, waits for the load to fall before each stream, and prints
`perf stat -e instructions:u,cycles:u` beside the times, with the medians of
three rounds; wall-time medians drift with the load, the counts do not. The
times are of runs without perf and the counts of one more run of each under
it: its counting lengthens each waking of a thread, 0.15 ms a picture on four
threads on the lab, which the tables before 2026-10-24 carry in their
four-thread columns, and the counts it does not change.

## The steps that got here

Each step was kept on the streams there were when it was made, and until
2026-10-12 those were "the capture", a 1600×1000 Mac capture most of whose
pictures turned out to decode to the picture before them, and "the video",
an x265 camera video, under Bun. So every step has been measured again on
the fourteen captures, under V8 on the idle VM: the module built from each
step's commit, all of them decoding every capture to FFmpeg's digests, then
timed alternately through one afternoon on the eight cells here, the
last row that evening. In
M cycles a picture on one thread at 2x unless said, and the median ms of a
strip on four threads, each row against the one above it:

| | commit | a still page | typing | the scroll | the fast scroll | the video | the video at 1x | 4 threads, ms: a still page at 1x | the video |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| The decoder as first written | `b9082b2` | 16.7 | 21.5 | 32.7 | 48.8 | 98.5 | 42.6 | 0.90 | 4.96 |
| SIMD128 kernels and a 16-bit inverse transform | `c77190e` | 16.0 | 18.9 | 25.6 | 37.3 | 55.5 | 26.1 | 0.89 | 3.50 |
| The transform by dot products, a wavefront that spins before it sleeps | `30f74c9` | 10.5 | 12.7 | 19.3 | 30.8 | 47.8 | 22.0 | 0.64 | 3.00 |
| One bounds check per kernel, padded CABAC contexts | `ad52364` | 10.0 | 12.1 | 18.7 | 30.1 | 47.3 | 21.8 | 0.62 | 3.03 |
| The filters in the wavefront, SAO in place, samples straight from the filters | `e4090e8` | 8.0 | 10.3 | 16.2 | 27.5 | 39.6 | 19.7 | 0.48 | 2.52 |
| Transform rows by shuffles, copies down columns, map fills by words | `0aa1fbd` | 6.5 | 8.5 | 14.0 | 24.6 | 35.8 | 17.9 | 0.37 | 2.28 |
| Boundary strengths as the blocks decode, the edge filter by vector, idle SAO blocks skipped | `8dbc348` | 5.5 | 7.4 | 12.6 | 22.2 | 34.2 | 17.7 | 0.29 | 2.22 |
| Each row starts as the first reference's rows | `79eeb4a` | 4.3 | 6.3 | 11.9 | 21.3 | 34.0 | 17.6 | 0.25 | 2.20 |
| The probable symbol's short path, the engine's registers, the merge list as far as its index | `38da479` | 3.6 | 5.4 | 10.9 | 20.1 | 32.6 | 16.6 | 0.22 | 2.06 |
| *no step: the filters reach the picture by pointer* | `4e95e0f` | 3.5 | 5.4 | 10.9 | 19.9 | 32.4 | 16.7 | 0.23 | 2.07 |
| The transform of a block with few coefficients over its coded columns | `2075045` | 3.5 | 4.9 | 10.8 | 20.1 | 31.9 | 16.6 | 0.21 | 2.08 |
| The sub-block walk in a function of its own, its contexts from a table | `d964e99` | 3.5 | 4.9 | 10.7 | 19.8 | 31.9 | 16.8 | 0.21 | 2.08 |
| The LPS table packed by model | `96a4144` | 3.5 | 4.8 | 10.6 | 19.8 | 32.2 | 16.6 | 0.22 | 2.02 |
| A row copies only the blocks changed since its buffer's picture | `8f1e91c` | 3.5 | 4.9 | 10.7 | 19.7 | 32.0 | 16.5 | 0.21 | 2.08 |
| A block with no edge strength is not scanned by the deblocking | `8a77392` | 3.4 | 4.7 | 10.5 | 19.7 | 31.8 | 16.7 | 0.19 | 2.10 |
| *no step: a display in strips put together* | `1c8b0f5` | 3.3 | 4.7 | 10.6 | 19.8 | 31.9 | 16.7 | 0.20 | 2.04 |
| The coefficients listed as they are parsed | `e631224` | 3.3 | 4.6 | 10.5 | 19.8 | 31.3 | 16.5 | 0.19 | 2.05 |
| The second pass by multiply-add, the angular turn by vector | `cb0221e` | 3.4 | 4.6 | 10.4 | 19.3 | 30.1 | 16.0 | 0.19 | 2.03 |
| An intra block's neighbours gathered as two runs | `02e1a8f` | 3.3 | 4.6 | 10.1 | 18.1 | 29.4 | 15.8 | 0.19 | 1.96 |
| Buffers kept for a display in strips | `1520dc8` | 2.2 | 3.6 | 9.9 | 18.1 | 29.3 | 15.8 | 0.19 | 2.04 |
| The calling thread is one of the wavefront's | `8180421` | 2.3 | 3.6 | 9.9 | 18.2 | 29.3 | 15.8 | 0.16 | 1.97 |
| A row of SAO's edge offset out of line, its offsets saturating bytes | `6ca65d7` | 2.1 | 3.6 | 9.8 | 18.0 | 29.0 | 15.7 | 0.16 | 1.97 |
| A buffer kept only for a reference that left a block of it; release 0.0.10 | `cd88a8a` | 2.2 | 3.7 | 9.8 | 18.0 | 28.9 | 15.7 | 0.17 | 1.97 |
| A coded sub-block decoded in the walk's function, not one of its own | `9071df3` | 2.1 | 3.5 | 9.7 | 17.9 | 28.5 | 15.4 | 0.17 | 1.93 |
| The deblocking filters take eight lines of an edge where they took four | `767b06f` | 2.1 | 3.6 | 9.6 | 17.4 | 28.1 | 15.2 | 0.16 | 1.88 |
| A wide block's residual added sixteen samples at a time | `87e32e6` | 2.2 | 3.5 | 9.6 | 17.5 | 28.5 | 15.1 | 0.16 | 1.87 |
| SAO's band offset a row in a function of its own, one table for its four bands | `48a662f` | 2.1 | 3.6 | 9.6 | 17.4 | 28.3 | 15.4 | 0.17 | 1.94 |
| A picture held until the page releases it, and where it changed | `ac4efe4` | 2.1 | 3.6 | 9.5 | 17.3 | 28.4 | 15.2 | 0.16 | 1.90 |
| The module as it is: a light strip at 1x on the calling thread alone, the pool asleep | `fc013d7` | 2.1 | 3.4 | 9.5 | 17.2 | 28.2 | 15.3 | 0.11 | 1.86 |

The first nine steps, the ones that made the decoder, each take cycles off
every capture: together the video at 2x goes from 98.5 M cycles a picture
to 32.6 M, a still strip from 16.7 M to 3.6 M, and the filters in the
wavefront alone take a quarter off a light strip's time on four threads.
Of the later ones the transform over the coded columns, the coefficient
list, the second pass, the gathering, the buffers kept, the calling thread,
the sub-block decoded in the walk's function, the deblocking's eight
lines and the light strip left to the calling thread each show the gain
they were kept for, on the streams they were aimed
at; the deblocking's skip and SAO's two rows are a digit on theirs. The
transform is the one that did not show under Bun, where no stream but the
typing took fewer cycles for it and the video 2% to 4% more; under V8 it is
9% fewer on the typing at 2x and 1% to 2% fewer on the video. Three show
nothing:

- **The sub-block walk in a function of its own** is level on every stream
  and 1% more on the video at 1x, as it was under Bun.
- **A row copying only the changed blocks** was a twelfth of the x265
  video's cycles. On the strips it is level, since no strip ever found a
  buffer holding its reference's ancestor; it is the buffers kept, six
  steps later, that it pays through.
- **The LPS table packed by model** was 4% of the capture's cycles under
  Bun. Under V8 it is a digit on the typing and the scroll and 1% more on
  the video.

And one goes the other way: **a wide block's residual added sixteen samples
at a time** is 1% more on the video at 2x here, level elsewhere, where it
was 1% fewer on the workstation.

The walk and the transform have each also been measured against their plain
form in the module as it is ([what remains](remaining.md#speed)): both
stay. The counts are printed to 0.1 M, a twentieth of a light strip's, so a
row that differs from the one above by that digit on a light stream may not
differ at all; on the video at 2x the module as it is and the step before
it, which it does not differ from on one thread, 28.2 M and 28.4 M, show what the digit is worth there.

As each step was measured when it was made, on the capture and the x265
video:

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
And three measured on the strips: the second pass of an interpolation by one
multiply-add over two rows interleaved, 4% of the video's cycles at 2x; an
angular prediction along the left column turned into place by vector, 4% of
the fast scroll's; and an intra block's neighbours gathered as two runs from
its corner with no flag per sample, 4.5% of the fast scroll's again.
And the buffers of a display in strips: a strip's reference is that strip of
the frame before, and the buffer free by then was another strip's, so no
buffer was ever reused and every strip was copied whole from its reference.
A free buffer that some reference descends from is now left for the picture
decoded from that reference, and a new one made in its place: a third of a
still strip's cycles at 2x and a fifth of the typing's, a sixth and a tenth
at 1x, 7% of the loop's, and 1% more on the scrolls and the video, whose
changed blocks are now copied by the run and not by the row.
And the thread that asks for a picture decoding its rows with the pool's,
where it had handed all of them over and slept until they were done: the
first row is under way while the others wake, 0.1 to 0.2 ms sooner for every
strip on four threads, which is two fifths of a still strip's time at 1x and
a fortieth of the video's at 2x.
