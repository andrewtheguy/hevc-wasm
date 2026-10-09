import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { accessUnits } from "./annexb";
import { type Fixture, type FixtureName, loadFixtures } from "./fixtures";
import { decodeStrip, decodeUnit, type LoadedRust, loadRust, type RustDecoder, type RustPicture, rustPictureMd5, rustPictureRows } from "./rust";

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
  for (const name of ["mac-330x194", "mac-352x256", "mac-still-352x256"] as const) {
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

describe("a display in strips", () => {
  // The fixture's pictures stand for strips, as the Mac orders them: a
  // keyframe is strip 0's, and the strips follow it in turn. The display is
  // 40 rows short of four strips, as the Mac's last strip runs past the end.
  const STRIPS = 4;
  const fixture = () => fixtures["mac-352x256"];
  const rows = () => STRIPS * fixture().height - 40;

  /** Each unit's strip: 0 at a keyframe, then in turn. */
  function strips(): { data: Uint8Array; strip: number; keyframe: boolean }[] {
    let since = 0;
    return accessUnits(fixture().stream).map((unit) => {
      since = unit.keyframe ? 0 : since + 1;
      return { data: unit.data, strip: since % STRIPS, keyframe: unit.keyframe };
    });
  }

  for (const threads of [1, POOL]) {
    test(`is its strips' pictures one under another, whole once every strip has come since the keyframe, on ${threads} thread(s)`, () => {
      const { width, height } = fixture();
      withDecoder(1, (alone) =>
        withDecoder(threads, (d) => {
          const expected = [0, 1, 2].map(() => new Uint8Array(width * rows()));
          let placed = 0;
          let shown = 0;
          for (const unit of strips()) {
            const part = rustPictureRows(loaded, decodeUnit(loaded, alone, unit.data)!);
            const at = unit.strip * height * width;
            expected.forEach((plane, i) => plane.set(part[i]!.subarray(0, plane.length - at), at));
            placed = (unit.keyframe ? 0 : placed) | (1 << unit.strip);
            const display = decodeStrip(loaded, d, unit.data, unit.strip, rows());
            if (placed !== 15) {
              expect(display).toBeNull();
              continue;
            }
            shown++;
            expect([display!.width, display!.height, display!.format]).toEqual([width, rows(), 2]);
            expect(rustPictureRows(loaded, display!)).toEqual(expected);
            // The strips after the first are predicted pictures here, not the
            // intra ones of the Mac's keyframe.
            expect(display!.keyframe).toBe(false);
          }
          expect(shown).toBeGreaterThan(0);
        }),
      );
    });
  }

  test("is refused of a height its strips are not quarters of, and of a fifth strip", () => {
    const { height } = fixture();
    const [unit] = strips();
    for (const [strip, displayRows] of [[0, STRIPS * height + 1], [0, (STRIPS - 1) * height - 1], [STRIPS, rows()]] as const) {
      withDecoder(1, (d) => {
        expect(() => decodeStrip(loaded, d, unit!.data, strip, displayRows)).toThrow(/not strip/);
      });
    }
    // In bounds, but not the quarter of the rows rounded up to 16.
    withDecoder(1, (d) => {
      expect(() => decodeStrip(loaded, d, unit!.data, 0, (STRIPS - 1) * height)).toThrow(/not strip/);
    });
  });

  test("a decoder goes from strips to whole pictures and back", () => {
    const units = strips();
    withDecoder(1, (d) => {
      const whole = (unit: { data: Uint8Array }) => {
        const picture = decodeUnit(loaded, d, unit.data)!;
        expect(picture.height).toBe(fixture().height);
      };
      for (const unit of units.slice(0, 4)) decodeStrip(loaded, d, unit.data, unit.strip, rows());
      whole(units[4]!);
      whole(units[5]!);
      // The strips from before the whole picture are gone: the display is
      // whole again after four new ones, and holds those alone.
      const { width, height } = fixture();
      const expected = [0, 1, 2].map(() => new Uint8Array(width * rows()));
      withDecoder(1, (alone) => {
        for (const unit of units.slice(0, 6)) decodeUnit(loaded, alone, unit.data);
        for (const [i, unit] of units.slice(6, 10).entries()) {
          const part = rustPictureRows(loaded, decodeUnit(loaded, alone, unit.data)!);
          const at = unit.strip * height * width;
          expected.forEach((plane, c) => plane.set(part[c]!.subarray(0, plane.length - at), at));
          const display = decodeStrip(loaded, d, unit.data, unit.strip, rows());
          if (i < 3) {
            expect(display).toBeNull();
          } else {
            expect(rustPictureRows(loaded, display!)).toEqual(expected);
          }
        }
      });
    });
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

describe("a damaged stream", () => {
  // The module's SIMD loops run nowhere else than here: the Rust fuzz target
  // (rust/hevc/fuzz) runs their scalar counterparts. So the fixtures are
  // damaged the ways a stream gets damaged, the same ways every run, and the
  // module must throw an error rather than trap, and decode each unit alike
  // on one thread and on the pool.
  const ROUNDS = 64;

  /** xorshift32, as a draw of `0..n`. */
  function rng(seed: number): (n: number) => number {
    let s = seed >>> 0 || 1;
    return (n) => {
      s ^= s << 13;
      s >>>= 0;
      s ^= s >>> 17;
      s ^= s << 5;
      s >>>= 0;
      return s % n;
    };
  }

  /** A copy of `stream` with flipped bits, a run of random bytes, a cut, or a unit dropped. */
  function damage(stream: Uint8Array, draw: (n: number) => number): Uint8Array {
    const out = stream.slice();
    switch (draw(4)) {
      case 0:
        for (let n = draw(8) + 1; n > 0; n--) {
          const at = draw(out.length);
          out[at] = (out[at] ?? 0) ^ (1 << draw(8));
        }
        return out;
      case 1: {
        const at = draw(out.length);
        const end = Math.min(at + draw(64) + 1, out.length);
        for (let i = at; i < end; i++) out[i] = draw(256);
        return out;
      }
      case 2:
        return out.subarray(0, draw(out.length));
      default: {
        const units = accessUnits(out);
        const unit = units[draw(units.length)]!.data;
        const start = unit.byteOffset;
        const rest = new Uint8Array(out.length - unit.length);
        rest.set(out.subarray(0, start));
        rest.set(out.subarray(start + unit.length), start);
        return rest;
      }
    }
  }

  /** The picture's digest, null for none, or "error": what the two decoders must agree on. */
  function outcome(decoder: RustDecoder, unit: Uint8Array): string | null {
    try {
      const picture = decodeUnit(loaded, decoder, unit);
      return picture && rustPictureMd5(loaded, picture);
    } catch (error) {
      expect(error).not.toBeInstanceOf(WebAssembly.RuntimeError);
      return "error";
    }
  }

  for (const name of ["mac-330x194", "mac-352x256"] as const) {
    test(`${name} never traps, and decodes alike on one thread and ${POOL}`, () => {
      const draw = rng(name.length);
      for (let round = 0; round < ROUNDS; round++) {
        const stream = damage(fixtures[name].stream, draw);
        const units = accessUnits(stream).map((u) => u.data);
        if (units.length === 0) units.push(stream);
        withDecoder(1, (one) =>
          withDecoder(POOL, (many) => {
            for (const unit of units) {
              expect([round, outcome(many, unit)]).toEqual([round, outcome(one, unit)]);
            }
          }),
        );
      }
    });
  }
});
