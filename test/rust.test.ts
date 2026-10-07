import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { accessUnits } from "./annexb";
import { type Fixture, type FixtureName, loadFixtures } from "./fixtures";
import { decodeUnit, type LoadedRust, loadRust, type RustDecoder, type RustPicture, rustPictureMd5 } from "./rust";

// The pool remotex would start, at most.
const POOL = 4;

let loaded: LoadedRust;
let fixtures: Record<FixtureName, Fixture>;

beforeAll(async () => {
  [loaded, fixtures] = await Promise.all([loadRust(POOL), loadFixtures()]);
});

afterAll(() => loaded?.close());

function withDecoder<T>(threads: number, fn: (decoder: RustDecoder) => T): T {
  const decoder = new loaded.glue.Decoder(threads);
  try {
    return fn(decoder);
  } finally {
    decoder.free();
  }
}

/** Decodes every unit of a fixture, as the page feeds them, reading each picture before the next unit. */
function decodeAll(decoder: RustDecoder, fixture: Fixture): { pictures: RustPicture[]; md5s: string[] } {
  const pictures: RustPicture[] = [];
  const md5s: string[] = [];
  for (const unit of accessUnits(fixture.stream)) {
    const picture = decodeUnit(loaded, decoder, unit.data);
    if (!picture) continue;
    pictures.push(picture);
    md5s.push(rustPictureMd5(loaded, picture));
  }
  return { pictures, md5s };
}

describe("the Mac's shape of stream", () => {
  for (const name of ["mac-330x194", "mac-352x256"] as const) {
    for (const threads of [1, POOL]) {
      test(`${name} decodes bit for bit as FFmpeg does, on ${threads} thread(s)`, () => {
        const fixture = fixtures[name];
        const { pictures, md5s } = withDecoder(threads, (d) => decodeAll(d, fixture));
        expect(md5s).toEqual(fixture.md5s);
        expect(pictures[0]!.keyframe).toBe(true);
        expect(pictures[1]!.keyframe).toBe(false);
        for (const p of pictures) {
          expect([p.width, p.height, p.format]).toEqual([fixture.width, fixture.height, 2]);
        }
      });
    }
  }

  test("the planes are in the module's memory, rows at their stride", () => {
    const fixture = fixtures["mac-330x194"];
    withDecoder(1, (d) => {
      const [unit] = accessUnits(fixture.stream);
      const picture = decodeUnit(loaded, d, unit!.data)!;
      for (const plane of picture.planes) {
        expect(plane.stride).toBeGreaterThanOrEqual(fixture.width);
        expect(plane.address + plane.stride * fixture.height).toBeLessThanOrEqual(loaded.memory.buffer.byteLength);
      }
    });
  });

  test("a stream joined before its keyframe decodes nothing until one arrives", () => {
    const fixture = fixtures["mac-352x256"];
    const units = accessUnits(fixture.stream);
    const second = units.findIndex((u, i) => i > 0 && u.keyframe);
    withDecoder(1, (d) => {
      // Leading pictures without their parameter sets or keyframe: nothing.
      for (const unit of units.slice(1, second)) {
        expect(decodeUnit(loaded, d, unit.data)).toBeNull();
      }
      const md5s: string[] = [];
      for (const unit of units.slice(second)) {
        const picture = decodeUnit(loaded, d, unit.data);
        if (picture) md5s.push(rustPictureMd5(loaded, picture));
      }
      expect(md5s).toEqual(fixture.md5s.slice(second));
    });
  });

  test("two decoders decode side by side", () => {
    const a = fixtures["mac-330x194"];
    const b = fixtures["mac-352x256"];
    withDecoder(POOL, (da) =>
      withDecoder(1, (db) => {
        const ua = accessUnits(a.stream);
        const ub = accessUnits(b.stream);
        const got = { a: [] as string[], b: [] as string[] };
        for (let i = 0; i < Math.max(ua.length, ub.length); i++) {
          const pa = ua[i] && decodeUnit(loaded, da, ua[i]!.data);
          const pb = ub[i] && decodeUnit(loaded, db, ub[i]!.data);
          if (pa) got.a.push(rustPictureMd5(loaded, pa));
          if (pb) got.b.push(rustPictureMd5(loaded, pb));
        }
        expect(got.a).toEqual(a.md5s);
        expect(got.b).toEqual(b.md5s);
      }),
    );
  });
});

describe("what it refuses", () => {
  test("a stream of another shape, by name", () => {
    withDecoder(1, (d) => {
      const [unit] = accessUnits(fixtures["yuv420p-330x194"].stream);
      expect(() => decodeUnit(loaded, d, unit!.data)).toThrow(/4:4:4/);
    });
  });

  test("garbage, and then goes on from the next keyframe", () => {
    const fixture = fixtures["mac-330x194"];
    const units = accessUnits(fixture.stream);
    withDecoder(1, (d) => {
      const garbage = new Uint8Array([0, 0, 1, 0x26, 1, 0xaf, 0x13, 0x55, 0, 0, 1, 0x02, 1, 0xff, 0x00, 0x00]);
      expect(() => decodeUnit(loaded, d, garbage)).toThrow();
      expect(() => decodeUnit(loaded, d, new Uint8Array([1, 2, 3, 4, 5]))).toThrow(/no NAL/);
      expect(decodeUnit(loaded, d, new Uint8Array(0))).toBeNull();
      const md5s: string[] = [];
      for (const unit of units) {
        const picture = decodeUnit(loaded, d, unit.data);
        if (picture) md5s.push(rustPictureMd5(loaded, picture));
      }
      expect(md5s).toEqual(fixture.md5s);
    });
  });
});
