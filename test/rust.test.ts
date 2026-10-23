import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { accessUnits } from "./annexb";
import { type Fixture, type FixtureName, loadFixtures } from "./fixtures";
import { decodeHeld, decodeUnit, type LoadedRust, loadRust, type RustDecoder, type RustPicture, rustChanged, rustPictureCopy, rustPictureMd5 } from "./rust";

// The pool zdesk would start, at most.
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

describe("a picture held", () => {
  // The page reads a picture while the next units decode. The fixture's
  // pictures stand for a display's strips, as the Mac orders them, each one's
  // reference the same strip of the frame before: three held at once here,
  // released out of order.
  for (const threads of [1, POOL]) {
    test(`is good until it is released, while the units after it decode, on ${threads} thread(s)`, () => {
      const fixture = fixtures["mac-352x256"];
      withDecoder(threads, (d) => {
        const held: RustPicture[] = [];
        const md5s: string[] = [];
        for (const unit of accessUnits(fixture.stream)) {
          const picture = decodeHeld(loaded, d, unit.data);
          if (picture) held.push(picture);
          if (held.length === 3) {
            // The oldest is read after the middle one is gone.
            const [oldest, middle] = held.splice(0, 2);
            const middleMd5 = rustPictureMd5(loaded, middle!);
            d.release(middle!.serial);
            md5s.push(rustPictureMd5(loaded, oldest!), middleMd5);
            d.release(oldest!.serial);
          }
        }
        for (const picture of held) {
          md5s.push(rustPictureMd5(loaded, picture));
          d.release(picture.serial);
        }
        expect(md5s).toEqual(fixture.md5s);
        // A serial not held is nothing.
        d.release(0);
        d.release(1 << 30);
      });
    });
  }

  test("once the last is released, picture() describes none, though an earlier one is still held", () => {
    const fixture = fixtures["mac-352x256"];
    withDecoder(1, (d) => {
      const held: RustPicture[] = [];
      for (const unit of accessUnits(fixture.stream)) {
        const picture = decodeHeld(loaded, d, unit.data);
        if (picture) held.push(picture);
        if (held.length === 2) break;
      }
      const [earlier, latest] = held;
      d.release(latest!.serial);
      expect([...new Int32Array(loaded.memory.buffer, d.picture(), 16)]).toEqual(new Array(16).fill(0));
      expect(rustPictureMd5(loaded, earlier!)).toBe(fixture.md5s[0]!);
      d.release(earlier!.serial);
    });
  });
});

describe("where a picture changed", () => {
  // Each picture against each of the four before it, as a strip is against its
  // strip of the frame before: every sample outside the rectangle is that
  // picture's.
  for (const name of ["mac-352x256", "mac-still-352x256"] as const) {
    for (const threads of [1, POOL]) {
      test(`${name}: every sample that differs is in the rectangle, on ${threads} thread(s)`, () => {
        const fixture = fixtures[name];
        const { width: w, height: h } = fixture;
        // How many rectangles were the whole picture, part of it, and none of it.
        const counts = { whole: 0, part: 0, none: 0 };
        withDecoder(threads, (d) => {
          const before: { serial: number; planes: Uint8Array[] }[] = [];
          for (const unit of accessUnits(fixture.stream)) {
            const picture = decodeUnit(loaded, d, unit.data);
            if (!picture) continue;
            const planes = rustPictureCopy(loaded, picture);
            // A picture's own serial, and none at all, say nothing of where.
            expect(rustChanged(loaded, d, 0)).toEqual({ x: 0, y: 0, width: w, height: h });
            for (const { serial, planes: was } of before) {
              const r = rustChanged(loaded, d, serial);
              expect(r.x >= 0 && r.y >= 0 && r.x + r.width <= w && r.y + r.height <= h).toBe(true);
              if (picture.keyframe) expect(r).toEqual({ x: 0, y: 0, width: w, height: h });
              if (r.width === w && r.height === h) counts.whole++;
              else if (r.width === 0) counts.none++;
              else counts.part++;
              for (let p = 0; p < 3; p++) {
                for (let y = 0; y < h; y++) {
                  for (let x = 0; x < w; x++) {
                    const inside = x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
                    if (!inside && planes[p]![y * w + x] !== was[p]![y * w + x]) {
                      throw new Error(`picture ${picture.serial} differs from ${serial} at (${x}, ${y}) of plane ${p}, outside ${JSON.stringify(r)}`);
                    }
                  }
                }
              }
            }
            before.unshift({ serial: picture.serial, planes });
            before.length = Math.min(before.length, 4);
          }
        });
        // The still screen's patch is a part of it.
        if (name === "mac-still-352x256") expect(counts.part).toBeGreaterThan(0);
      });
    }
  }
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
