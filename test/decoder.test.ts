import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { type AccessUnit, accessUnits, isVcl, splitNals } from "./annexb";
import { type Fixture, type FixtureName, KEYINT, loadFixtures } from "./fixtures";
import { Decoder, type HevcModule, type LoadedModule, loadHevc, type Picture, pictureMd5 } from "./hevc";

// The pool the page instantiates the module with, remotex's largest.
const POOL = 8;

let loaded: LoadedModule;
let hevc: HevcModule;
let fixtures: Record<FixtureName, Fixture>;

beforeAll(async () => {
  [loaded, fixtures] = await Promise.all([loadHevc(POOL), loadFixtures()]);
  hevc = loaded.module;
});

afterAll(() => loaded?.close());

interface Decoded {
  results: number[];
  pictures: Picture[];
  md5s: string[];
}

// Feeds each unit as the page does, reading the picture before the next call
// releases it.
function decodeUnits(decoder: Decoder, units: AccessUnit[]): Decoded {
  const out: Decoded = { results: [], pictures: [], md5s: [] };
  for (const unit of units) {
    const result = decoder.decode(unit.data, unit.keyframe);
    out.results.push(result);
    if (result !== 1) continue;
    const picture = decoder.picture();
    out.pictures.push(picture);
    if (picture.format !== -1) out.md5s.push(pictureMd5(decoder.heap, picture));
  }
  return out;
}

function withDecoder<T>(threads: number, fn: (decoder: Decoder) => T): T {
  const decoder = new Decoder(hevc, threads);
  try {
    return fn(decoder);
  } finally {
    decoder.destroy();
  }
}

describe("module", () => {
  test("exports the C surface and its memory", () => {
    for (const name of ["_hevc_create", "_hevc_input", "_hevc_decode", "_hevc_picture", "_hevc_destroy"] as const) {
      expect(typeof hevc[name]).toBe("function");
    }
    expect(hevc.wasmMemory.buffer).toBeInstanceOf(SharedArrayBuffer);
  });

  test("creates and destroys decoders, and destroying null is a no-op", () => {
    for (const threads of [1, 4, POOL]) {
      const pointer = hevc._hevc_create(threads);
      expect(pointer).not.toBe(0);
      hevc._hevc_destroy(pointer);
    }
    hevc._hevc_destroy(0);
  });

  test("hevc_input returns a buffer in linear memory for the unit's size", () => {
    withDecoder(1, (decoder) => {
      const input = hevc._hevc_input(decoder.pointer, 1 << 20) >>> 0;
      expect(input).toBeGreaterThan(0);
      expect(input + (1 << 20)).toBeLessThanOrEqual(hevc.wasmMemory.buffer.byteLength);
    });
  });
});

describe("decodes bit-identically to native FFmpeg", () => {
  const cases: [FixtureName, 0 | 1 | 2][] = [
    ["yuv444p-330x194", 2],
    ["yuv420p-330x194", 0],
    ["yuv422p-320x180", 1],
    ["yuv444p-wpp-ctu16", 2],
    ["yuv420p-slices4", 0],
  ];
  for (const [name, format] of cases) {
    for (const threads of [1, 4, POOL]) {
      test(`${name}, ${threads} thread${threads > 1 ? "s" : ""}`, () => {
        const fixture = fixtures[name];
        const units = accessUnits(fixture.stream);
        const decoded = withDecoder(threads, (decoder) => decodeUnits(decoder, units));
        // One unit in, its picture out, from the same call.
        expect(decoded.results).toEqual(units.map(() => 1));
        for (const picture of decoded.pictures) {
          expect(picture).toMatchObject({ width: fixture.width, height: fixture.height, format });
          const chromaWidth = format === 2 ? fixture.width : Math.ceil(fixture.width / 2);
          expect(picture.planes[0].stride).toBeGreaterThanOrEqual(fixture.width);
          expect(picture.planes[1].stride).toBeGreaterThanOrEqual(chromaWidth);
          expect(picture.planes[2].stride).toBeGreaterThanOrEqual(chromaWidth);
        }
        expect(decoded.md5s).toEqual(fixture.md5s);
      });
    }
  }
});

describe("picture", () => {
  test("marks the pictures of IRAP units as keyframes", () => {
    const units = accessUnits(fixtures["yuv444p-330x194"].stream);
    const decoded = withDecoder(4, (decoder) => decodeUnits(decoder, units));
    expect(decoded.pictures.map((p) => p.keyframe)).toEqual(units.map((u) => u.keyframe));
    expect(units.filter((u) => u.keyframe).length).toBe(Math.ceil(units.length / KEYINT));
  });

  test("reports more than eight bits as a format a VideoFrame cannot hold", () => {
    const fixture = fixtures["yuv420p10-320x180"];
    const units = accessUnits(fixture.stream);
    const decoded = withDecoder(4, (decoder) => decodeUnits(decoder, units));
    expect(decoded.results).toEqual(units.map(() => 1));
    for (const picture of decoded.pictures) {
      expect(picture).toMatchObject({ width: 320, height: 180, format: -1 });
    }
  });

  // AVColorRange, AVColorSpace, AVColorPrimaries, AVColorTransferCharacteristic.
  test.each([
    ["bt709-tv", { range: 1, colorspace: 1, primaries: 1, transfer: 1 }],
    ["bt601-pc", { range: 2, colorspace: 5, primaries: 5, transfer: 6 }],
    ["yuv420p-160x96", { range: 1, colorspace: 2, primaries: 2, transfer: 2 }],
  ] as const)("reports the color description of %s", (name, color) => {
    const units = accessUnits(fixtures[name].stream);
    const decoded = withDecoder(1, (decoder) => decodeUnits(decoder, units));
    for (const picture of decoded.pictures) expect(picture).toMatchObject(color);
  });
});

describe("stream handling", () => {
  test("a unit of parameter sets alone completes no picture", () => {
    const fixture = fixtures["yuv444p-330x194"];
    const [first, ...rest] = accessUnits(fixture.stream);
    const firstVcl = splitNals(first!.data).find((n) => isVcl(n.type))!;
    const parameterSets = first!.data.subarray(0, firstVcl.start);
    const picture = first!.data.subarray(firstVcl.start);
    withDecoder(4, (decoder) => {
      expect(decoder.decode(parameterSets, false)).toBe(0);
      expect(decoder.decode(picture, true)).toBe(1);
      const md5s = [pictureMd5(decoder.heap, decoder.picture()), ...decodeUnits(decoder, rest).md5s];
      expect(md5s).toEqual(fixture.md5s);
    });
  });

  test("follows a change of size and chroma format at a keyframe", () => {
    const a = fixtures["yuv444p-330x194"];
    const b = fixtures["yuv420p-160x96"];
    const c = fixtures["yuv444p-wpp-ctu16"];
    const units = [a, b, c].flatMap((f) => accessUnits(f.stream));
    const decoded = withDecoder(4, (decoder) => decodeUnits(decoder, units));
    expect(decoded.results).toEqual(units.map(() => 1));
    expect(decoded.md5s).toEqual([...a.md5s, ...b.md5s, ...c.md5s]);
    expect(decoded.pictures.at(a.md5s.length)).toMatchObject({ width: 160, height: 96, format: 0 });
    expect(decoded.pictures.at(-1)).toMatchObject({ width: 352, height: 288, format: 2 });
  });

  test("rejects an empty unit and carries on", () => {
    const fixture = fixtures["yuv420p-160x96"];
    withDecoder(1, (decoder) => {
      expect(decoder.decode(new Uint8Array(0))).toBeLessThan(0);
      expect(decodeUnits(decoder, accessUnits(fixture.stream)).md5s).toEqual(fixture.md5s);
    });
  });

  test("survives garbage, and decodes from the next keyframe", () => {
    const fixture = fixtures["yuv420p-160x96"];
    let seed = 1;
    const noise = Uint8Array.from({ length: 4096 }, () => (seed = (seed * 1103515245 + 12345) >>> 0) >>> 24);
    const garbage = [
      noise,
      // Start codes and slice headers in front of noise.
      new Uint8Array([0, 0, 1, 0x02, 0x01, 0x80, ...noise.subarray(0, 64)]),
      new Uint8Array([0, 0, 0, 1, 0x26, 0x01, 0xaf, ...noise.subarray(64, 256)]),
    ];
    withDecoder(4, (decoder) => {
      for (const unit of garbage) expect(decoder.decode(unit, true)).toBeLessThanOrEqual(1);
      expect(decodeUnits(decoder, accessUnits(fixture.stream)).md5s).toEqual(fixture.md5s);
    });
  });

  test("joining mid-stream, decodes bit-identically from the next keyframe", () => {
    const fixture = fixtures["yuv444p-330x194"];
    const units = accessUnits(fixture.stream);
    expect(units[KEYINT]!.keyframe).toBe(true);
    withDecoder(4, (decoder) => {
      // Pictures whose references never arrived: whatever comes out, no crash.
      decodeUnits(decoder, units.slice(1, KEYINT));
      const decoded = decodeUnits(decoder, units.slice(KEYINT));
      expect(decoded.results).toEqual(units.slice(KEYINT).map(() => 1));
      expect(decoded.md5s).toEqual(fixture.md5s.slice(KEYINT));
    });
  });

  test("decoders on one module keep their own state", () => {
    const a = fixtures["yuv444p-330x194"];
    const b = fixtures["yuv420p-slices4"];
    const unitsA = accessUnits(a.stream);
    const unitsB = accessUnits(b.stream);
    const da = new Decoder(hevc, 1);
    const db = new Decoder(hevc, 1);
    try {
      const md5sA: string[] = [];
      const md5sB: string[] = [];
      for (let i = 0; i < Math.max(unitsA.length, unitsB.length); i++) {
        for (const [decoder, unit, md5s] of [[da, unitsA[i], md5sA], [db, unitsB[i], md5sB]] as const) {
          if (!unit) continue;
          expect(decoder.decode(unit.data, unit.keyframe)).toBe(1);
          md5s.push(pictureMd5(decoder.heap, decoder.picture()));
        }
      }
      expect(md5sA).toEqual(a.md5s);
      expect(md5sB).toEqual(b.md5s);
    } finally {
      da.destroy();
      db.destroy();
    }
  });
});
