# Benchmarks

What the decoder measures against FFmpeg and against the release before, how
a change is measured, and what each step that got it here was worth. Where a
picture's time goes inside the decoder, by function and by count, is in
[the architecture](architecture.md#where-the-time-goes); what is left to
gain, in [what remains](remaining.md).

## Against FFmpeg and release 0.0.9

The comparison is of the module under Bun, as it is and as release 0.0.9,
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
only the strips that changed. Each ratio is the module's ms over FFmpeg's
beside it, under 1.00 where the module is the quicker; it sets a median
against a mean, so a few long pictures weigh on FFmpeg's figure and not on
the module's. On four threads FFmpeg is run with `-thread_type slice`, its
threads on the rows of one picture as the module's are, so both figures are
how long a strip takes; left to itself it decodes four pictures at once,
which is quicker over a file and no use to a display that is watched. Per
picture, lower better:

| 1440×240, a strip | pictures | 1 thread, M cycles | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,450 | 1.5 | 1.8 | 2.5 | 0.17 | 0.85 | $\color{green}{0.20}$ | 0.29 | 0.90 | $\color{green}{0.32}$ |
| A window dragged | 2,403 | 2.0 | 2.0 | 2.6 | 0.35 | 0.88 | $\color{green}{0.40}$ | 0.47 | 0.83 | $\color{green}{0.57}$ |
| A quarter of the display in a loop | 2,273 | 2.5 | 2.7 | 3.2 | 0.70 | 1.08 | $\color{green}{0.65}$ | 0.70 | 0.86 | $\color{green}{0.81}$ |
| Typing in a terminal | 2,248 | 2.1 | 2.3 | 3.0 | 0.18 | 1.02 | $\color{green}{0.18}$ | 0.39 | 0.80 | $\color{green}{0.49}$ |
| Chrome scrolling an article | 2,588 | 4.8 | 4.7 | 4.9 | 1.02 | 1.62 | $\color{green}{0.63}$ | 1.06 | 1.16 | $\color{green}{0.91}$ |
| The same four times as fast | 2,362 | 9.5 | 9.5 | 8.8 | 2.59 | 2.85 | $\color{green}{0.91}$ | 1.60 | 1.67 | $\color{green}{0.96}$ |
| A video over the whole display | 1,978 | 20.5 | 20.3 | 17.1 | 6.33 | 5.44 | $\color{red}{1.16}$ | 2.91 | 2.49 | $\color{red}{1.17}$ |

| 2880×464, a strip | pictures | 1 thread, M cycles | 0.0.9 | FFmpeg | 1 thread, ms | FFmpeg | Ratio | 4 threads, ms | FFmpeg by rows | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| A still page | 1,694 | 2.8 | 4.3 | 9.0 | 0.53 | 2.90 | $\color{green}{0.18}$ | 0.56 | 1.36 | $\color{green}{0.41}$ |
| A window dragged | 2,507 | 5.8 | 5.9 | 9.7 | 1.49 | 3.12 | $\color{green}{0.48}$ | 0.98 | 1.41 | $\color{green}{0.70}$ |
| A quarter of the display in a loop | 2,230 | 7.2 | 7.7 | 11.3 | 2.36 | 3.87 | $\color{green}{0.61}$ | 1.42 | 1.69 | $\color{green}{0.84}$ |
| Typing in a terminal | 2,424 | 4.9 | 6.1 | 10.8 | 0.67 | 3.73 | $\color{green}{0.18}$ | 0.67 | 1.47 | $\color{green}{0.46}$ |
| Chrome scrolling an article | 2,579 | 13.2 | 13.0 | 15.6 | 3.17 | 4.97 | $\color{green}{0.64}$ | 1.94 | 2.35 | $\color{green}{0.83}$ |
| The same four times as fast | 2,501 | 23.7 | 23.4 | 24.1 | 6.80 | 7.64 | $\color{green}{0.89}$ | 3.29 | 3.51 | $\color{green}{0.94}$ |
| A video over the whole display | 2,446 | 39.9 | 39.5 | 34.1 | 12.28 | 10.76 | $\color{red}{1.14}$ | 4.55 | 3.82 | $\color{red}{1.19}$ |

On one thread the module takes fewer cycles than native FFmpeg wherever most
of a picture is the picture before, which it neither decodes nor filters nor,
since 0.0.9, copies: two fifths fewer on the still page at 1x and two thirds
at 2x, a third and a half on the typing, and a fifth to two fifths on the
drag and the loop; 15% fewer on the scroll at 2x. Where a picture is new it
takes more: 8% on the fast scroll at 1x, and 20% and 17% on the video, whose
strip at 2x is 12 ms on one thread, four of them more than the 33 ms of a
frame at thirty a second, and 4.6 ms on four.

On four threads, strip for strip, the module is the quicker on all but the
video: by half and more on the still page and the typing, by a sixth to two
fifths on the drag and the loop, and by 4% to 17% on the scrolls. The video
is 17% and 19% behind, further than on one thread. A light strip still gains
nothing from threads, whose waking costs more than its few rows do to
decode: a still strip at 1x is 0.17 ms on one thread and 0.29 ms on four in
the module, 0.85 ms and 0.90 ms in FFmpeg. The four-thread columns are of a
later run than the others, made when the calling thread became one of the
four.

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

How the decoder got here, each step measured as it was made and on the
streams there were then: "the capture" is a 1600×1000 Mac capture
most of whose pictures turned out to decode to the picture before them, and
"the video" the x265 camera video. None of these shares has been measured
again on the strips, where one of the steps did not happen at all until
buffers were kept for them, the last step below.

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
