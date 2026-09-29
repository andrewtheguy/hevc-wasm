import { describe, expect, test } from "bun:test";

import { accessUnits, NAL, splitNals } from "./annexb";
import { loadFixtures } from "./fixtures";

const IDR_W_RADL = 19;
const TRAIL_R = 1;

// A NAL unit behind a start code: a two-byte header (layer 0, temporal id 0),
// then for a slice, first_slice_segment_in_pic_flag.
function nal(type: number, { first = true, long = false } = {}): number[] {
  const startCode = long ? [0, 0, 0, 1] : [0, 0, 1];
  const body = type < 32 ? [first ? 0x80 : 0x00, 0xaa] : [0xaa];
  return [...startCode, type << 1, 0x01, ...body];
}

describe("splitNals", () => {
  test("finds three- and four-byte start codes, and each unit's type", () => {
    const stream = new Uint8Array([...nal(NAL.VPS, { long: true }), ...nal(NAL.SPS), ...nal(IDR_W_RADL)]);
    const nals = splitNals(stream);
    expect(nals.map((n) => n.type)).toEqual([NAL.VPS, NAL.SPS, IDR_W_RADL]);
    expect(nals[0]).toMatchObject({ start: 0, header: 4 });
    expect(nals[1]!.start).toBe(nals[0]!.end);
    expect(nals.at(-1)!.end).toBe(stream.length);
  });

  test("finds nothing without a start code", () => {
    expect(splitNals(new Uint8Array([1, 2, 3, 0, 0, 2]))).toEqual([]);
  });
});

describe("accessUnits", () => {
  test("parameter sets and SEI begin the unit of the picture they precede", () => {
    const stream = new Uint8Array([
      ...nal(NAL.VPS), ...nal(NAL.SPS), ...nal(NAL.PPS), ...nal(NAL.PREFIX_SEI), ...nal(IDR_W_RADL),
      ...nal(NAL.PREFIX_SEI), ...nal(TRAIL_R),
      ...nal(NAL.AUD), ...nal(TRAIL_R),
    ]);
    const units = accessUnits(stream);
    expect(units.map((u) => u.nals)).toEqual([
      [NAL.VPS, NAL.SPS, NAL.PPS, NAL.PREFIX_SEI, IDR_W_RADL],
      [NAL.PREFIX_SEI, TRAIL_R],
      [NAL.AUD, TRAIL_R],
    ]);
    expect(units.map((u) => u.keyframe)).toEqual([true, false, false]);
    expect(Buffer.concat(units.map((u) => u.data))).toEqual(Buffer.from(stream));
  });

  test("a picture's later slices and suffix SEI stay in its unit", () => {
    const stream = new Uint8Array([
      ...nal(TRAIL_R), ...nal(TRAIL_R, { first: false }), ...nal(NAL.SUFFIX_SEI), ...nal(NAL.EOS),
      ...nal(TRAIL_R),
    ]);
    expect(accessUnits(stream).map((u) => u.nals)).toEqual([
      [TRAIL_R, TRAIL_R, NAL.SUFFIX_SEI, NAL.EOS],
      [TRAIL_R],
    ]);
  });

  test("splits every fixture into as many units as native FFmpeg decodes pictures", async () => {
    for (const fixture of Object.values(await loadFixtures())) {
      const units = accessUnits(fixture.stream);
      expect(units.length, fixture.name).toBe(fixture.md5s.length);
      expect(units[0]!.keyframe, fixture.name).toBe(true);
    }
  });
});
