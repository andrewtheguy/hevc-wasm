# Benchmarks

What the decoder measures against FFmpeg and against the release before, how
a change is measured, and what each step that got it here was worth. Where a
picture's time goes inside the decoder, by function and by count, is in
[the architecture](architecture.md#where-the-time-goes); what is left to
gain, in [what remains](remaining.md).

## Against FFmpeg and release 0.0.9

The comparison is of the module under Bun, as release 0.0.10, which is
before the one change since ([below](#the-steps-that-got-here)), and as
release 0.0.9,
against FFmpeg's native decoder: Debian 13's ffmpeg 7.1.5-0+deb13u1, built by
gcc 14 with `--toolchain=hardened`, which picks its vector code as it runs,
AVX2 on this i5-8500T, run as `ffmpeg -threads N -i FILE -benchmark -f null -`.
They ran on a six-core x86 workstation, alternately on the same cores, one
for one thread and four for four, and the medians of three rounds are shown.
The instruction and cycle counts are `perf stat`'s for the whole process and
do not depend on the load. The module's ms is the median of its own clock's
per picture, and FFmpeg's its process's wall time over the pictures. The
streams are seven scenes of the display, each at 1x and at 2x, thirty seconds
of each; a picture is a strip, a quarter of the display, and the Mac sends
only the strips that changed. Strips are what remotex asks of every such
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

| 1440×240, a strip | pictures | 1 thread, M cycles | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,450 | 1.5 | 1.8 | 2.4 | 0.17 | 0.80 | $\color{green}{0.21}$ | 0.29 | 0.80 | $\color{green}{0.36}$ |
| A window dragged | 2,403 | 2.0 | 2.0 | 2.6 | 0.35 | 0.88 | $\color{green}{0.40}$ | 0.46 | 0.74 | $\color{green}{0.62}$ |
| A quarter of the display in a loop | 2,273 | 2.5 | 2.7 | 3.2 | 0.67 | 1.05 | $\color{green}{0.64}$ | 0.67 | 0.93 | $\color{green}{0.72}$ |
| Typing in a terminal | 2,248 | 2.1 | 2.3 | 3.0 | 0.17 | 0.99 | $\color{green}{0.17}$ | 0.38 | 0.78 | $\color{green}{0.49}$ |
| Chrome scrolling an article | 2,588 | 4.8 | 4.7 | 4.9 | 1.01 | 1.57 | $\color{green}{0.64}$ | 1.01 | 1.15 | $\color{green}{0.88}$ |
| The same four times as fast | 2,362 | 9.4 | 9.4 | 8.8 | 2.53 | 2.77 | $\color{green}{0.91}$ | 1.56 | 1.62 | $\color{green}{0.96}$ |
| A video over the whole display | 1,978 | 20.3 | 20.4 | 17.0 | 6.21 | 5.32 | $\color{red}{1.17}$ | 2.81 | 2.43 | $\color{red}{1.16}$ |

| 2880×464, a strip | pictures | 1 thread, M cycles | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,694 | 2.8 | 4.3 | 9.0 | 0.54 | 2.83 | $\color{green}{0.19}$ | 0.56 | 1.31 | $\color{green}{0.43}$ |
| A window dragged | 2,507 | 5.8 | 6.0 | 9.7 | 1.48 | 3.08 | $\color{green}{0.48}$ | 0.96 | 1.37 | $\color{green}{0.70}$ |
| A quarter of the display in a loop | 2,230 | 7.0 | 7.8 | 11.2 | 2.25 | 3.55 | $\color{green}{0.63}$ | 1.37 | 1.70 | $\color{green}{0.81}$ |
| Typing in a terminal | 2,424 | 4.7 | 6.0 | 10.7 | 0.50 | 3.37 | $\color{green}{0.15}$ | 0.65 | 1.46 | $\color{green}{0.45}$ |
| Chrome scrolling an article | 2,579 | 13.2 | 13.1 | 15.6 | 3.12 | 4.88 | $\color{green}{0.64}$ | 1.90 | 2.33 | $\color{green}{0.82}$ |
| The same four times as fast | 2,501 | 23.5 | 23.4 | 24.0 | 6.59 | 7.53 | $\color{green}{0.88}$ | 3.21 | 3.45 | $\color{green}{0.93}$ |
| A video over the whole display | 2,446 | 39.2 | 39.7 | 34.0 | 11.79 | 10.53 | $\color{red}{1.12}$ | 4.47 | 3.80 | $\color{red}{1.18}$ |

On one thread the module takes fewer cycles than native FFmpeg wherever most
of a picture is the picture before, which it neither decodes nor filters nor,
as release 0.0.9 still did, copies: two fifths fewer on the still page at 1x and two thirds
at 2x, a third and a half on the typing, and a fifth to two fifths on the
drag and the loop; 15% fewer on the scroll at 2x. Where a picture is new it
takes more: 7% on the fast scroll at 1x, and 19% and 15% on the video, whose
strip at 2x is 12 ms on one thread, four of them more than the 33 ms of a
frame at thirty a second, and 4.5 ms on four.

On four threads, strip for strip, the module is the quicker on all but the
video: by half and more on the still page and the typing, by a fifth to two
fifths on the drag and the loop, and by 4% to 18% on the scrolls. The video
is 16% and 18% behind. A light strip still gains nothing from threads, whose
waking costs more than its few rows do to decode: a still strip at 1x is
0.17 ms on one thread and 0.29 ms on four in the module, 0.80 ms on either
in FFmpeg. All of it is one run, the night of 2026-10-12. The host's
1-minute load as each stream's one-thread timing ended was 1.1 to 1.6, of
which the decoder being timed is 1.0; the cycle counts do not depend on the
rest and the milliseconds do.

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
three rounds; wall-time medians drift with the load, the counts do not.

## The steps that got here

Each step was kept on the streams there were when it was made, and until
2026-10-12 those were "the capture", a 1600×1000 Mac capture most of whose
pictures turned out to decode to the picture before them, and "the video",
an x265 camera video. So every step has been measured again on the fourteen
captures: the module built from each step's commit, all of them decoding
every capture to FFmpeg's digests, alternately through one night. In
M cycles a picture on one thread at 2x unless said, and the median ms of a
strip on four threads, each row against the one above it:

| | commit | a still page | typing | the scroll | the fast scroll | the video | the video at 1x | 4 threads, ms: a still page at 1x | the video |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| The decoder as first written | `b9082b2` | 16.2 | 23.1 | 39.1 | 61.4 | 130.8 | 57.4 | 1.86 | 15.02 |
| SIMD128 kernels and a 16-bit inverse transform | `c77190e` | 15.9 | 19.6 | 28.8 | 44.8 | 73.1 | 34.6 | 1.83 | 8.79 |
| The transform by dot products, a wavefront that spins before it sleeps | `30f74c9` | 15.3 | 18.3 | 26.0 | 40.1 | 66.4 | 29.2 | 1.69 | 7.70 |
| One bounds check per kernel, padded CABAC contexts | `ad52364` | 12.9 | 15.8 | 24.1 | 38.7 | 62.1 | 28.4 | 1.56 | 7.37 |
| The filters in the wavefront, SAO in place, samples straight from the filters | `e4090e8` | 10.6 | 13.2 | 20.8 | 35.1 | 51.7 | 25.5 | 0.67 | 5.72 |
| Transform rows by shuffles, copies down columns, map fills by words | `0aa1fbd` | 8.4 | 11.0 | 18.2 | 32.4 | 47.9 | 23.3 | 0.66 | 5.36 |
| Boundary strengths as the blocks decode, the edge filter by vector, idle SAO blocks skipped | `8dbc348` | 7.0 | 9.4 | 16.4 | 29.0 | 45.6 | 22.9 | 0.64 | 5.18 |
| Each row starts as the first reference's rows | `79eeb4a` | 5.4 | 7.8 | 15.5 | 27.8 | 45.3 | 23.0 | 0.56 | 5.20 |
| The probable symbol's short path, the engine's registers, the merge list as far as its index | `38da479` | 4.7 | 6.9 | 14.1 | 25.9 | 43.3 | 21.4 | 0.52 | 5.00 |
| *no step: the filters reach the picture by pointer* |  | 4.6 | 6.7 | 13.9 | 25.6 | 42.4 | 21.0 | 0.49 | 4.90 |
| The transform of a block with few coefficients over its coded columns | `2075045` | 4.6 | 6.5 | 14.0 | 25.9 | 43.1 | 21.8 | 0.48 | 5.05 |
| The sub-block walk in a function of its own, its contexts from a table | `d964e99` | 4.6 | 6.5 | 14.1 | 26.0 | 43.6 | 22.1 | 0.50 | 5.08 |
| The LPS table packed by model | `96a4144` | 4.6 | 6.4 | 13.9 | 25.7 | 43.2 | 21.7 | 0.48 | 5.03 |
| A row copies only the blocks changed since its buffer's picture | `8f1e91c` | 4.7 | 6.5 | 14.0 | 26.0 | 43.3 | 21.8 | 0.50 | 5.06 |
| A block with no edge strength is not scanned by the deblocking | `8a77392` | 4.4 | 6.2 | 13.8 | 25.7 | 43.4 | 21.7 | 0.48 | 5.07 |
| *no step: a display in strips put together* |  | 4.4 | 6.2 | 13.8 | 25.7 | 43.5 | 21.7 | 0.48 | 5.09 |
| The coefficients listed as they are parsed | `e631224` | 4.4 | 6.1 | 13.6 | 25.4 | 42.0 | 21.1 | 0.48 | 4.92 |
| The second pass by multiply-add, the angular turn by vector | `cb0221e` | 4.4 | 6.0 | 13.3 | 24.6 | 40.2 | 20.6 | 0.48 | 4.70 |
| An intra block's neighbours gathered as two runs | `02e1a8f` | 4.3 | 6.0 | 13.1 | 23.4 | 39.7 | 20.4 | 0.47 | 4.68 |
| Buffers kept for a display in strips | `1520dc8` | 2.8 | 4.8 | 13.3 | 23.8 | 39.6 | 20.4 | 0.50 | 4.71 |
| The calling thread is one of the wavefront's | `8180421` | 2.9 | 4.7 | 13.2 | 23.7 | 39.6 | 20.4 | 0.30 | 4.53 |
| A row of SAO's edge offset out of line, its offsets saturating bytes | `6ca65d7` | 2.8 | 4.7 | 13.1 | 23.6 | 39.1 | 20.3 | 0.30 | 4.42 |
| A buffer kept only for a reference that left a block of it | `cd88a8a` | 2.8 | 4.7 | 13.2 | 23.5 | 39.2 | 20.3 | 0.29 | 4.47 |

The first nine steps, the ones that made the decoder, each take cycles off
every capture: together the video at 2x goes from 130.8 M
cycles a picture to 43.3 M, a still strip from 16.2 M to 4.7 M, and the
filters in the wavefront alone take three fifths off a light strip's time on
four threads. Of the later ones the deblocking's skip, the coefficient list,
the packed LPS table, the second pass, the gathering, the buffers kept and
the calling thread each show the gain they were kept for, on the streams
they were aimed at. Three do not:

- **The transform over the coded columns** was a fifth of the capture's
  instructions and a sixth of its cycles. On the captures no stream but the
  typing at 2x takes fewer cycles for it, 3%, and the video takes 2% to 4%
  more.
- **The sub-block walk in a function of its own** is level on every stream
  and 1% more on the video.
- **A row copying only the changed blocks** was a twelfth of the x265
  video's cycles. On the strips it was up to 5% more, since no strip ever
  found a buffer holding its reference's ancestor; it is the buffers kept,
  six steps later, that it pays through.

Each of the first two has since been measured against its plain form in the
module as it is ([what remains](remaining.md#speed)): both stay, the
transform being worth 8% of the typing's cycles at 2x there, and a coded
sub-block is decoded in the walk's function where it had one of its own, 1%
to 2% fewer cycles on the scrolls and the video, which the tables above, of
the night before, do not have. The counts are printed to 0.1 M, a
twentieth of a light strip's, so a row that differs from the one above by
that digit on a light stream may not differ at all.

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
