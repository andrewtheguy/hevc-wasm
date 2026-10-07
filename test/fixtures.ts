// Synthetic HEVC streams in test/data, with native FFmpeg's decode of each as
// the reference: every picture's MD5 from `ffmpeg -f framemd5`, which the wasm
// decoder must match bit for bit. Both are committed, so a run needs neither
// ffmpeg nor libx265, and an encoder upgrade cannot change what is tested.
// `bun run fixtures` regenerates them from SPECS (test/generate-fixtures.ts).
//
// Streams have no B-frames, as the Mac's has none, so every access unit returns
// its picture from the same call.

import { existsSync } from "node:fs";
import { join } from "node:path";

export interface FixtureSpec {
  size: string;
  pixFmt: string;
  frames?: number;
  // A lavfi graph of the size to encode, in place of testsrc2 at that size.
  source?: string;
  // Extra x265 parameters, after bframes=0 and keyint=6.
  x265?: string;
}

export interface Fixture {
  name: string;
  stream: Uint8Array;
  width: number;
  height: number;
  pixFmt: string;
  // The reference decode's MD5 of each picture, in output order.
  md5s: string[];
}

// What test/data/reference.json holds for each stream.
export interface Reference {
  sha256: string;
  width: number;
  height: number;
  pixFmt: string;
  md5s: string[];
}

export interface ReferenceFile {
  // `ffmpeg -version`'s first line, for the encoder and the reference decoder.
  ffmpeg: string;
  fixtures: Record<string, Reference>;
}

export const DATA_DIR = join(import.meta.dir, "data");
export const REFERENCE = join(DATA_DIR, "reference.json");

export const KEYINT = 6;

// The Mac's shape of stream, as x265 comes closest to it: 4:4:4 at 8 bits, one
// slice per picture with its coding tree block rows as a wavefront, 32-pixel
// coding tree blocks, I and P pictures with two short-term references, SAO and
// per-block QP, and none of the tools the Mac leaves off. What the pure-Rust
// decoder (rust/) takes; the FFmpeg one takes it too.
export const MAC_X265 = [
  "profile=main444-8", "wpp=1", "pools=4", "frame-threads=1", "ctu=32", "min-cu-size=8", "ref=2",
  "no-weightp=1", "signhide=0", "strong-intra-smoothing=0", "amp=0", "tskip=0", "temporal-mvp=0",
  "scenecut=0", "open-gop=0", "sao=1", "aq-mode=1", "rect=1", "tu-intra-depth=1", "tu-inter-depth=2",
  "repeat-headers=1", "no-info=1",
].join(":");

export const SPECS = {
  // Sizes off the 8-pixel grid, so the SPS crops with a conformance window.
  "yuv444p-330x194": { size: "330x194", pixFmt: "yuv444p" },
  // The Mac's shape: a cropped size, and one a whole number of 32-pixel rows.
  "mac-330x194": { size: "330x194", pixFmt: "yuv444p", x265: MAC_X265 },
  "mac-352x256": { size: "352x256", pixFmt: "yuv444p", x265: MAC_X265 },
  // A screen mostly still: a patch moves across one frozen picture, and the
  // rest is skipped from the picture before, which the decoder leaves in the
  // buffer it reuses rather than copy.
  "mac-still-352x256": {
    size: "352x256",
    pixFmt: "yuv444p",
    source: "testsrc2=size=352x256:rate=30,trim=end_frame=1,loop=loop=-1:size=1[bg];testsrc2=size=44x36:rate=30[fg];[bg][fg]overlay=x=37+9*n:y=50+5*n",
    x265: MAC_X265,
  },
  "yuv420p-330x194": { size: "330x194", pixFmt: "yuv420p" },
  "yuv422p-320x180": { size: "320x180", pixFmt: "yuv422p" },
  // Wavefront rows, many of them with 16-pixel CTBs, as the Mac sends.
  "yuv444p-wpp-ctu16": { size: "352x288", pixFmt: "yuv444p", x265: "wpp=1:ctu=16" },
  "yuv420p-slices4": { size: "320x240", pixFmt: "yuv420p", x265: "slices=4:ctu=16" },
  "yuv420p10-320x180": { size: "320x180", pixFmt: "yuv420p10le", frames: 4 },
  "yuv420p-160x96": { size: "160x96", pixFmt: "yuv420p", frames: 4 },
  "bt709-tv": {
    size: "160x96",
    pixFmt: "yuv444p",
    frames: 2,
    x265: "range=limited:colormatrix=bt709:colorprim=bt709:transfer=bt709",
  },
  "bt601-pc": {
    size: "160x96",
    pixFmt: "yuv444p",
    frames: 2,
    x265: "range=full:colormatrix=bt470bg:colorprim=bt470bg:transfer=smpte170m",
  },
} satisfies Record<string, FixtureSpec>;

export type FixtureName = keyof typeof SPECS;

export const streamPath = (name: string) => join(DATA_DIR, `${name}.h265`);

let fixtures: Promise<Record<FixtureName, Fixture>> | null = null;

export function loadFixtures(): Promise<Record<FixtureName, Fixture>> {
  fixtures ??= (async () => {
    if (!existsSync(REFERENCE)) throw new Error(`no ${REFERENCE}: run \`bun run fixtures\``);
    const reference = (await Bun.file(REFERENCE).json()) as ReferenceFile;
    const entries = await Promise.all(
      Object.keys(SPECS).map(async (name) => {
        const ref = reference.fixtures[name];
        const path = streamPath(name);
        if (!ref || !existsSync(path)) throw new Error(`no fixture ${name}: run \`bun run fixtures\``);
        const stream = new Uint8Array(await Bun.file(path).arrayBuffer());
        // A stream regenerated without its reference, or the other way round.
        const sha256 = new Bun.CryptoHasher("sha256").update(stream).digest("hex");
        if (sha256 !== ref.sha256) throw new Error(`${path} does not match reference.json: run \`bun run fixtures\``);
        const fixture: Fixture = { name, stream, width: ref.width, height: ref.height, pixFmt: ref.pixFmt, md5s: ref.md5s };
        return [name, fixture] as const;
      }),
    );
    return Object.fromEntries(entries) as Record<FixtureName, Fixture>;
  })();
  return fixtures;
}
