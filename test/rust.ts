// Loads the decoder's module as released (build/out, built by ./build.sh) under
// Node or Bun as zDesk's decode worker loads it: the module instantiated on a
// shared memory, its pool's threads started as workers that each run an instance
// of it, and a picture read from where the decoder left its planes. Node's APIs
// only, which Bun has too, so the tests run under Bun and the benchmark under
// Node, whose V8 is the engine of the Chrome the module is for.

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Worker } from "node:worker_threads";

export const OUT_DIR = resolve(process.env.HEVC_WASM_DIR ?? fileURLToPath(new URL("../build/out", import.meta.url)));

/** What rust/hevc-web/src/lib.rs exports, as wasm-bindgen's glue presents it. */
interface Glue {
  default(options: { module_or_path: Uint8Array | WebAssembly.Module; memory?: WebAssembly.Memory }): Promise<{ memory: WebAssembly.Memory }>;
  module(): WebAssembly.Module;
  startPool(threads: number): void;
  Decoder: new (threads: number) => RustDecoder;
}

export interface RustDecoder {
  input(size: number): number;
  decode(): boolean;
  picture(): number;
  changed(since: number): number;
  release(serial: number): void;
  free(): void;
}

/** A picture as the module describes it: the planes are in `memory`. */
export interface RustPicture {
  width: number;
  height: number;
  format: number;
  range: number;
  matrix: number;
  primaries: number;
  transfer: number;
  planes: { address: number; stride: number }[];
  keyframe: boolean;
  /** Which of the decoder's pictures it is, for `release`. */
  serial: number;
}

export interface LoadedRust {
  glue: Glue;
  memory: WebAssembly.Memory;
  /** Stops the pool's workers, which would otherwise keep the process alive. */
  close(): void;
}

/** What each pool worker is told: the glue to import, the compiled module, and the one memory. */
export interface PoolSeat {
  js: string;
  module: WebAssembly.Module;
  memory: WebAssembly.Memory;
}

/** Instantiates the module and starts a pool of `threads` workers. Once per process. */
export async function loadRust(threads: number): Promise<LoadedRust> {
  const js = `${OUT_DIR}/hevc.js`;
  const wasm = `${OUT_DIR}/hevc.wasm`;
  if (!existsSync(js) || !existsSync(wasm)) {
    throw new Error(`no hevc.js and hevc.wasm in ${OUT_DIR}: run ./build.sh, or set HEVC_WASM_DIR`);
  }
  const glue = (await import(pathToFileURL(js).href)) as Glue;
  const bytes = new Uint8Array(await readFile(wasm));
  const { memory } = await glue.default({ module_or_path: bytes });
  const seat: PoolSeat = { js, module: glue.module(), memory };
  const workers: Worker[] = [];
  try {
    await Promise.all(
      Array.from({ length: threads }, () => new Promise<void>((resolve, reject) => {
        const worker = new Worker(new URL("./rust-pool.worker.ts", import.meta.url));
        workers.push(worker);
        worker.once("message", (data: string | null) => (data === null ? resolve() : reject(new Error(data))));
        worker.once("error", (error: Error) => reject(error));
        worker.postMessage(seat);
      })),
    );
  } catch (error) {
    // A seat not taken: the others would keep the process alive.
    for (const worker of workers) worker.terminate();
    throw error;
  }
  glue.startPool(threads);
  return {
    glue,
    memory,
    close() {
      for (const worker of workers) worker.terminate();
    },
  };
}

// The picture each decoder last gave `decodeUnit`, which the next call releases.
const last = new WeakMap<RustDecoder, number>();

/**
 * Feeds `unit` to `decoder` and reads the picture it decoded to, if any: good until the next call for the decoder,
 * which releases it, as a page done with each picture before the next would.
 */
export function decodeUnit(loaded: LoadedRust, decoder: RustDecoder, unit: Uint8Array): RustPicture | null {
  const before = last.get(decoder);
  if (before !== undefined) decoder.release(before);
  last.delete(decoder);
  const picture = decodeHeld(loaded, decoder, unit);
  if (picture) last.set(decoder, picture.serial);
  return picture;
}

/** Feeds `unit` to `decoder` and reads the picture it decoded to, if any, which is held until released. */
export function decodeHeld(loaded: LoadedRust, decoder: RustDecoder, unit: Uint8Array): RustPicture | null {
  const at = decoder.input(unit.length);
  new Uint8Array(loaded.memory.buffer, at, unit.length).set(unit);
  if (!decoder.decode()) return null;
  const p = new Int32Array(loaded.memory.buffer, decoder.picture(), 16);
  return {
    width: p[0]!,
    height: p[1]!,
    format: p[2]!,
    range: p[3]!,
    matrix: p[4]!,
    primaries: p[5]!,
    transfer: p[6]!,
    planes: [0, 1, 2].map((i) => ({ address: p[7 + i]!, stride: p[10 + i]! })),
    keyframe: p[13] === 1,
    serial: p[14]!,
  };
}

/** `ffmpeg -f framemd5`'s digest of the picture: its three planes, rows packed. */
export function rustPictureMd5(loaded: LoadedRust, picture: RustPicture): string {
  const heap = new Uint8Array(loaded.memory.buffer);
  const hasher = createHash("md5");
  for (const plane of picture.planes) {
    for (let y = 0; y < picture.height; y++) {
      hasher.update(heap.subarray(plane.address + y * plane.stride, plane.address + y * plane.stride + picture.width));
    }
  }
  return hasher.digest("hex");
}

/** A rectangle of a picture's planes, as `changed` gives it: a width of 0 where nothing changed. */
export interface RustRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Where the picture `decoder` last decoded to may differ from picture `since`. */
export function rustChanged(loaded: LoadedRust, decoder: RustDecoder, since: number): RustRect {
  const [x, y, width, height] = new Int32Array(loaded.memory.buffer, decoder.changed(since), 4);
  return { x: x!, y: y!, width: width!, height: height! };
}

/** The picture's three planes, rows packed, copied out of the module's memory. */
export function rustPictureCopy(loaded: LoadedRust, picture: RustPicture): Uint8Array[] {
  const heap = new Uint8Array(loaded.memory.buffer);
  return picture.planes.map((plane) => {
    const copy = new Uint8Array(picture.width * picture.height);
    for (let y = 0; y < picture.height; y++) {
      copy.set(heap.subarray(plane.address + y * plane.stride, plane.address + y * plane.stride + picture.width), y * picture.width);
    }
    return copy;
  });
}
