# Architecture

hevc-wasm is a software decoder for the HEVC a High Performance Mac sends,
written in Rust for that one shape of stream and nothing else of HEVC, and
compiled to WebAssembly with SIMD128 and threads for
[remotex](https://github.com/andrewtheguy/remotex)'s browser client. It
decodes bit for bit as FFmpeg does. The [README](../README.md) is the
operator's and builder's view of the same thing: building, testing,
releasing and the measurements against the release before.
[What remains](remaining.md) is the list of what it does not do yet.

## Data path

```text
remotex's decode worker (frontend/src/hevcWasm.worker.ts)
   │  input(size)   the access unit's bytes go into the module's memory
   │  decode()      true when the unit completed a picture; throws when it does not decode
   │  decodeStrip() the same for a strip of a display: true when the display is whole
   │  picture()     sixteen numbers: size, layout, colour, each plane's address and stride, keyframe
   ▼
hevc-web, the wasm-bindgen module ── hevc::Decoder
   NAL units → SPS, PPS → slice header → reference picture set
   → the picture's coding tree block rows, as a wavefront on the pool's threads:
       CABAC → coding quadtree → prediction (intra, or merge and motion vectors from one reference)
       → residual → samples in place; deblocking a block behind, SAO a block behind that
   → three planes of bytes at the coded size, the window to show, the colour, keyframe or not
   ▼
remotex's paint worker uploads the planes to WebGL from the module's shared memory
```

A picture's planes stay in the module's memory until the next `input`, which
is what lets the page read them without a copy.

## The stream

The decoder reads what the Mac sends: 4:4:4 at 8 bits, one slice per
picture with its coding tree block rows coded as a wavefront
(`entropy_coding_sync_enabled_flag`), I and P pictures with short-term
references, and nothing else. Every other shape is refused by name, as an
`Unsupported` error, rather than decoded wrong:

- From the parameter sets: a chroma format other than 4:4:4, separate colour
  planes, a bit depth other than 8, picture reordering, scaling lists,
  asymmetric motion partitions, PCM, long-term reference pictures, strong
  intra smoothing, SPS and PPS extensions, dependent slice segments,
  `pic_output_flag`, sign data hiding, `cabac_init_flag`, constrained intra
  prediction, transform skip, weighted prediction, transquant bypass, tiles,
  a stream without the wavefront, reference list modification, and a picture
  larger than level 6.2 allows (35,651,584 luma samples), the bound every
  allocation is made under.
- From the slice header: more than one slice per picture, B slices, temporal
  motion vector prediction, and a slice whose entry points are not one per
  coding tree block row.
- From the NAL header: any picture type other than the two IDR types and the
  two trailing types. NAL units of other layers are skipped.

An access unit is Annex B: its NAL units with their start codes, parameter
sets before the slice. The decoder takes one unit per call and returns its
picture, if the unit had one. A stream is joined at an IDR: units before the
first one decode to nothing. A new SPS of another size takes effect at the
next IDR; a non-IDR picture whose size differs from its references' is
refused.

### A display in strips

Offered four tiles (`tilesPerFrame` 4 in remotex's offer), the Mac sends the
display as four strips: each the
display's whole width and a quarter of its height rounded up to a multiple of
16, lying top to bottom a strip's height apart, so the last runs past the
display's last row. They are not HEVC's tiles. The strips are pictures of one
stream with one set of parameter sets, which give the strip's size, and one
decoding order: a keyframe is an IDR of strip 0 and an intra picture of each
other strip, and a picture after it may refer to an earlier one of any strip. So
one decoder takes every strip's access units in that order, as it takes a
stream of whole pictures, and nothing of the decoding differs.

Which strip a picture is, and how many rows the display has, are not in the
stream: the Mac sends each strip under an RTP source of its own, and the
parameter sets know only the strip. The caller gives both with the unit, and
`strips` copies the decoded strip to its place in the display, three planes
of the display's size. A frame carries the strips that changed and no
others, so the display is whole once every strip has come since the keyframe
and stays whole from then on, each later strip changing its own rows. A
keyframe starts the display over, since what the other strips hold is from
before whatever the keyframe mends. A picture whose height is not a quarter
of the display's rounded up to a multiple of 16 is
refused as invalid. A unit decoded whole, with `decode`, drops the display, so
strips after it start one over.

Remotex offers one tile for a display whose last strip would start past its
last row (heights under 48, 65 to 95 and 129 to 143), which the Mac's encoder
fails on in four; that stream is whole pictures, and `decode` takes it.

### What a failure means

`decode` returns an `Invalid` error for a malformed unit, or one that refers
to a picture the decoder does not have, and `Unsupported` for a stream of
another shape. Either leaves the decoder waiting for an IDR: whatever the
unit left half done is dropped, the references are released, and every
non-IDR picture until the next IDR decodes to nothing. A unit with bytes but
no NAL unit is invalid too, since a reference may have been lost with it; an
empty unit decodes to nothing and changes nothing.

The module turns the error into a thrown `Error` with the same message, and
remotex's worker reports it as the decoder's `EncodingError`; the page then
asks the Mac for a keyframe.

A WebAssembly trap, from a panic or an out-of-bounds access, would instead
be fatal to the module instance, which is why malformed input must come
back as an error. The decoder bounds its parsing on the slice data's end and
the maps' sizes, and checks the parameter sets' sizes before allocating for
them.

## The decoder crate: `rust/hevc`

| module | what it holds |
|---|---|
| `nal` | Annex B framing, the two-byte NAL unit header, emulation prevention |
| `bits` | an MSB-first bit reader with Exp-Golomb codes, over an RBSP |
| `ps` | the sequence and picture parameter sets, read whole and refused where they ask for a tool the decoder lacks |
| `slice` | the slice segment header of the one slice a picture has |
| `decoder` | parameter sets, the reference picture set and the DPB, one picture per access unit |
| `pic` | pictures, their planes, and the per-block maps of the one being decoded |
| `wavefront` | running the rows across threads, each two coding tree blocks behind the row above |
| `shared` | the planes and maps as the rows share them: the contract every `unsafe` access relies on |
| `cabac` | the arithmetic decoding engine and the context models |
| `ctu` | one coding tree block row from its substream: the coding quadtree, the coding unit, and the filters following behind |
| `ctu::residual` | the transform tree, residual coding, scaling, the inverse transform and the residual add |
| `ctu::inter` | the prediction unit, merge mode, the motion vector predictor, motion compensation from one reference |
| `intra` | intra sample prediction: the neighbours, substituted and filtered, then planar, DC or angular |
| `itx` | scaling and the inverse transforms, in 16 bits, as partial butterflies |
| `deblock` | the deblocking filter, a coding tree block at a time |
| `sao` | sample adaptive offset, in place, a coding tree block at a time |
| `strips` | the display put together from the four strips the Mac may send it in |
| `kernels` | the sample loops, plain Rust; `kernels::simd128` the same loops in 128-bit vectors |
| `tables` | the specification's constant tables, built at compile time |
| `error` | `Invalid` and `Unsupported` |

The decoding arithmetic was transcribed from
[rusty_h265](https://github.com/Remade-With-Rust/rusty_h265) 0.6.0
(Apache-2.0, `rust/hevc/LICENSE`), which decodes 4:2:0 only; the 4:4:4
handling, the byte planes and the threading are this repository's.

### Pictures and references

A picture is three planes of bytes at the coded size, all three the luma's
size, each row starting a multiple of 64 bytes apart so the kernels' vectors
never cross a row. The decoded picture comes back with the conformance window
to show and the colour the VUI states, or `2` (unspecified) where it does
not. Buffers are pooled: a picture no reference or output holds goes back to
the pool, four at most, still holding its picture (see the wavefront), and a
sequence of another size takes new ones.

References are short-term only, kept by picture order count. Each picture's
reference picture set decides what stays; a used reference the decoder does
not have is a grey picture, as FFmpeg makes one, so a dropped unit costs the
pictures until the next IDR rather than a trap; more references than the
stream's `max_dec_pic_buffering` is an error. The reference list is list 0
as §8.3.4 builds it, the earlier pictures then the later, repeated to the
slice's active count.

### The wavefront

Each coding tree block row is a substream of the slice, entered at its entry
point, and decodes on its own thread of rayon's pool, two coding tree blocks
behind the row above: a block waits until the row above has finished the
block up and to the right of it, which is what its prediction and its
context models need. The models start as the row above left them after its
second block (§9.3.1). Rows are claimed in order; a decoder made with one
thread runs them in order on the caller and needs no pool.

Each row's progress is a sequentially consistent counter the row below waits
on, spinning a few hundred times before it sleeps, since the block is
usually there or about to be. A row that fails marks itself failed, stops
the others, and the first error is the unit's. Every thread keeps a
scratch of the row's buffers, taken from a set of as many as there are
threads, any free one.

A P picture's rows start as the first reference's rows, one sequential copy
before the wait on the row above, so a skipped block with a zero motion
vector from that reference, which on a screen is nearly every block, is
already in place and costs nothing.

Where it can, the copy is not made either. Each picture keeps, per coding
tree block, whether it is still the picture its rows started as: no coding
unit of the block wrote a sample, no edge in it or along a side of it has a
boundary strength, so the deblocking changed nothing, and its SAO is off. A buffer in the pool still holds the picture it was decoded
as, known by a serial number, so a free buffer holding a picture the first
reference descends from, base by base through the pictures still referenced,
already is that reference wherever no picture on the way changed the block.
The decoder takes that buffer when there is one, and a row copies only the
runs of blocks in between. A buffer whose decode failed holds no picture and
is never taken for one.

### The filters, behind the wavefront

The in-loop filters run on the same threads a block or two behind the
decoding, in place, as FFmpeg's do: nothing waits for a pass over the whole
picture, and a block is filtered while it is still in cache.

- Deblocking (`deblock`) runs on a coding tree block once the blocks to its
  right, below and below-right are decoded, since they predict from its
  unfiltered samples. The boundary strength of every edge is settled as the
  blocks decode, into a per-4×4 map of two bits per edge, zero off the 8×8
  grid, so the filter is a scan of eight strengths at a time, nearly all
  zero, and the edge filter with its four lines in the vector's lanes. A
  coding tree block also records whether any edge along its left side, its
  top or within it has a strength: one without is not scanned, and the same
  flags say whether the deblocking reached into the block beside it. A
  block's vertical edges are filtered when it is reached and its horizontal
  edges eight columns behind, since the picture's vertical edges must all be
  filtered before any horizontal one that crosses them.
- SAO (`sao`) follows a block behind that, since it reads a block's
  deblocked neighbours; a block saves its last line and column, as
  deblocked, before filtering itself where a block below or to the right will
  read them. A block with no offset of its own and no neighbour's edge offset
  to save a line for costs one check, which on these streams is nearly all of
  them.

Between rows on different threads the wavefront's two blocks of lag keep
each row's filters behind the row above's decoding; the last row's thread
finishes the two rows above it as well.

### Sharing the picture between threads

`shared` holds the one contract every `unsafe` access in the crate relies
on. A row's thread writes only its own coding tree block row of the planes
and maps, and reads the row above only where that row has finished, which
the progress counter orders after the write. The filters touch a few lines
of the row above, from blocks the rows above are at least two blocks past.
So no two threads ever touch the same byte at once, and no reference made
to the planes outlives the access it serves.

The planes and maps are reached through raw pointers (`PlanePtr`, `MapPtr`).
The decoding kernels take a block as a strided slice, which stays within one
thread's rows; the filters, whose blocks share rows with other threads'
blocks, reach the picture by pointer and never hold a slice of it.

### Parsing

The arithmetic decoder is two registers, the offset and the range, kept in
locals; the data pointer, which only a refill touches, stays in memory with
the models. The bits read ahead sit in the offset's word under the offset,
carrying their own end marker rather than a count, so the test for a refill
is the lower half of the word being zero, and the slice data is padded so a
refill is one whole load with no call. A bin's range depends on the bin
before through the LPS it looks up by model and quartile of the range, and
that chain, not the instruction count, is what a bin costs: the four
quartiles' LPS of a model are packed in one word, so the lookup by model
starts before the range is known and the range selects a byte by a shift.
The most probable symbol, which most bins are, takes a short path: the
range shrinks by the other symbol's share and doubles at most once to put
it back. The walk over a block's sub-blocks, most of them not coded, is a
small function of its own, and each coded sub-block another, since V8
spills what a large one holds: the walk keeps the coded flags on a bordered
grid so a neighbour is a load, and takes the sixteen significance contexts
of a sub-block from a table built at compile time, by the block's size,
component and scan, the sub-block's place and its neighbours' flags; the
sub-block's significance flags are decoded unrolled by position, so each
flag's context and bit are constants of the code.
Coefficients are scaled as they are parsed and listed one after another,
each with its place, so the parser writes no block and nothing is cleared
for one; a coefficient's remaining level, the escape few of them have, is
read in a function apart, so its two loops of bypass bins are no part of
the sub-block's. The merge candidate list is built only as far as the coded index,
nearly always zero; the neighbours to the left and above, decoded before the
block wherever they are, are not looked up in the z-order map; and a
block's motion is one word, written once per 4×4.

### The transform

Scaling and the inverse transforms run in 16 bits: the scaled coefficients,
the intermediate between the two passes and the residual are all within 16
bits for 8-bit samples, and the sums of a pass are 32-bit, which is what the
kernels multiply into. Each pass is a partial butterfly: the even half of an
N-point transform is the N/2-point transform of the even coefficients, down
to the 4-point base, and each level adds its odd half, a sum over the odd
coefficients that were non-zero. The row pass splats each pair of
coefficients with one shuffle.

A block with few coefficients, which on the capture is nearly every 32×32
block (some seven, scattered so that their bounding box is most of the
block), takes a path in proportion to them instead, from the parser's list:
the coefficients are chained by their columns, and the first pass runs down
the coded columns only, each the sum of its chain's coefficients times their
rows of the matrix, and writes the columns in pairs, zipped a 32-bit lane
per row;
the second pass sums each row over those pairs by dot products against the
pairs' rows of the matrix zipped the same way, with no butterfly. The dense
path remains for a 4×4 block and for a block with more coefficients than
twice its width, or more than sixteen coded columns: the list is put into a
block of zeros, which is cleared behind the transform, row by coded row.

### The kernels

Every sample loop has two bodies with the same arithmetic in the same order:
plain Rust in `kernels`, and WebAssembly's 128-bit vectors in
`kernels::simd128`, which the public function dispatches to when the build
has `simd128`. A run on either path reconstructs the same picture; the
native build runs the plain ones. The vector kernels are the interpolation
filters, which write samples straight from their last pass, block copies
down sixteen-wide columns and across the rows of a run of coding tree blocks, the residual add, planar and angular intra
prediction, the inverse transform by dot products, the deblocking edge
filters, SAO's band and edge offsets, and the map fills by whole words
rather than `memory.fill` calls into the runtime.

## The module: `rust/hevc-web`

The page's module, in the shape of remotex's `egfx` compositor module, so the
two loaders differ only in where the glue comes from:

| export | what it does |
|---|---|
| `init({ module_or_path, memory? })` | wasm-bindgen's: instantiates the module, on the given shared memory when one is given, and returns `{ memory }` |
| `module()` | the compiled `WebAssembly.Module`, for a worker to make its instance of |
| `runPoolThread()` | runs one of the pool's threads on the worker that calls it: waits for the pool to be started, returns when the pool is gone, never in a page |
| `startPool(threads)` | makes rayon's pool of `threads` workers each already in `runPoolThread` |
| `new Decoder(threads)` | one stream's decoder, its rows on `threads` of the pool, or on the caller for one |
| `decoder.input(size)` | the address of `size` bytes for the next access unit; releases the previous picture |
| `decoder.decode()` | decodes the unit written there: true when it completed a picture; throws for a unit that does not decode |
| `decoder.decodeStrip(strip, rows)` | decodes the unit written there as strip `strip`, from 0 at the top, of a display of `rows` rows: true when the display has every strip since its keyframe; throws as `decode` does, and for a picture that is not a strip of such a display |
| `decoder.picture()` | the address of sixteen `i32`s describing the picture, or after `decodeStrip` the display |
| `decoder.free()` | wasm-bindgen's |

A page has no threads to spawn, so the pool's threads are seats: a worker of
the page imports the glue, instantiates the module on the one memory, posts
that it is ready, and calls `runPoolThread`, which blocks in the pool for
good; once every seat is taken, the main instance calls `startPool`. A pool
thread that does not start fails the module's load.

`picture` describes the picture in sixteen numbers, so the paint worker can
read the planes from the module's memory without a copy:

| index | value |
|---|---|
| 0, 1 | width and height, cropped to the window |
| 2 | the chroma layout, `2` for 4:4:4 |
| 3 | the colour range, `2` full or `1` limited, which an unstated range means |
| 4, 5, 6 | the matrix, primaries and transfer as the stream codes them, `2` for unstated |
| 7, 8, 9 | each plane's start in the module's memory, at the window's origin |
| 10, 11, 12 | each plane's stride |
| 13 | whether the picture is an IDR, where a stream can be joined; for a display, whether its strips are all the keyframe's or intra pictures since, so that nothing before the keyframe is in it |

A display's planes are the decoder's own and are written by the next
`decodeStrip`, so they are good until the next `input` as a picture's are.

The memory is shared and imported, so each thread's instance is given the
one memory, with a maximum of 1 GiB; the build is `+simd128,+atomics,
+bulk-memory` with the standard library built for them (`build-std`, which is
why the build is a nightly), and LLVM's output is shipped without `wasm-opt`,
which made the residual parser slower under V8. Panics abort.

Picture buffers are pooled and sized by the sequence; the input buffer grows
to the largest access unit. A 2880×1800 picture at 4:4:4 is 15.5 MB, and the
DPB keeps `max_dec_pic_buffering` of them plus the one being decoded.

## How remotex loads it

The release archive, `hevc-wasm-vX.Y.Z.tar.gz`, holds `hevc.js` and
`hevc.wasm`. remotex pins one release by version and SHA-256
(`src/hevc_wasm.rs`), reads the archive once at start-up, refuses any other
build, and serves the two files from memory at `/hevc/`, with the
cross-origin isolation the shared memory needs. The decode worker imports the
glue from there, instantiates the module, seats the pool's threads as workers
of the page's bundle (`hevcPool.worker.ts`), and drives a `Decoder` per
stream; the paint worker reads the planes where `picture` says they are.
The decoder's licence keeps it out of every remotex build, as the native
decoder's keeps that one out, so the archive, the pin and the `[hevc_wasm]`
table stay, and the two loaders stay two.

## The native harness: `rust/hevc-bench`

Decodes an Annex B file natively, one access unit at a time, and prints each
picture's MD5 as `ffmpeg -f framemd5` does, with the timing to stderr, so a
stream can be checked against FFmpeg's digests on any number of threads. It
exits non-zero for a file with no access units or a unit that does not
decode. `HEVC_DUMP=path:index` writes one picture as raw planar 4:4:4.

## Tests

`bun test` loads the released files under Bun as the page loads them: the
module on a shared memory, its pool's threads as workers. The two `mac-*`
fixtures, x265's nearest shape to the Mac's, must decode to native FFmpeg's
digests on one thread and four; the streams of other shapes must be refused
by name. The tests also cover where the planes are in the memory, joining a
stream before its keyframe, two decoders side by side, garbage units
with and without start codes, and a display in strips: a fixture's pictures
taken as strips, strip 0 at each keyframe, must come out as those pictures
one under another, cut at the display's last row. The fixtures and their reference are
committed, regenerated by `bun run fixtures` from `test/fixtures.ts`.

## Where the time goes

Profiled as the module under Node on one thread of an x86 workstation, with
`perf record` of V8's JIT output. The 1600×1000 Mac capture (`cap5`, 236
pictures), 13.6 ms a picture under the profiler:

| share | function | what it is |
|---|---|---|
| 23.8% | `sub_block` | one coded sub-block's significance flags, levels and signs |
| 21.7% | `inverse_transform` | the 32×32 transforms over their few coefficients, mostly the row pass |
| 13.9% | `sub_blocks` | the walk over a block's sub-blocks: a coded-sub-block flag each, 154 K a picture |
| 10.1% | `residual_block` | the last position, the block's setup, and the residual add inlined (5%) |
| 4.7% | libc `memmove` | the copy of the first reference each row starts as |
| 2.3% | `intra_predict` | |
| 2.2% | `edge_strengths` | deblocking boundary strengths |
| 1.9% | `coding_quadtree` | |
| 1.0% | `prediction_unit` | |
| 0.9% | `transform_tree` | |
| 0.9% | deblock `edges` | |
| 0.8% | `run_rows` | the wavefront's waits |

The 1080p camera video in the Mac's shape (`sample`, 2,896 pictures), 3.8 ms a
picture under the profiler:

| share | function |
|---|---|
| 14.2% | `sub_block` |
| 10.3% | `coding_quadtree` |
| 9.6% | `intra_predict` |
| 9.4% | `run_rows` (the copy of the changed blocks' runs each row starts with, and the wavefront's waits) |
| 7.5% | deblock `ctb` (the scan of the blocks with a strength, and the edge filters) |
| 7.4% | `prediction_unit` |
| 7.4% | `edge_strengths` |
| 5.9% | `copy_block` |
| 3.5% | libc `memmove` (two symbols: the rows copied whole and the stripe copies) |
| 2.9% | SAO `component` |
| 2.8% | `inverse_transform` |
| 2.5% | `residual_block` |
| 2.1% | `filter_after` |
| 1.7% | `merge_motion` |

Per picture of the capture: 4,060 residual blocks, 3,022 of them 32×32,
28.8 K coefficients in all (about seven per 32×32 block) in 27 K coded
sub-blocks of the 154 K the walk visits, 645 K context-coded bins and 46 K
bypass bins, so about 160 bins per 32×32 block, nearly all of them
coded-sub-block and significance flags. The video: 551 blocks, 8,244
coefficients, 45 K context bins, 19.6 K bypass bins, 4,029 coding units of
which 3,642 are skipped, 3,880 prediction units of which 3,774 are merges and
3,202 are still blocks from the first reference; of its 2,040 coding tree
blocks 1,709 have no edge with a strength, and in a P picture 1,436 are left
as the picture before and 1,220 are in the reused buffer already.

On four threads the capture decodes 2.6× as fast as on one; the video no
faster (1.9 ms on one, 2.3 ms on four), its cycles rising from 11 M to 13 M.
