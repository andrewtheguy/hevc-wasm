// Loads the decoder's module as released (build/out, built by ./build.sh) under
// Bun as remotex's decode worker loads it: the module instantiated on a shared
// memory, its pool's threads started as workers that each run an instance of it,
// and a picture read from where the decoder left its planes.

import { existsSync } from "node:fs";
import { resolve } from "node:path";

export const OUT_DIR = resolve(process.env.HEVC_WASM_DIR ?? `${import.meta.dir}/../build/out`);

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
  const glue = (await import(js)) as Glue;
  const bytes = new Uint8Array(await Bun.file(wasm).arrayBuffer());
  const { memory } = await glue.default({ module_or_path: bytes });
  const seat: PoolSeat = { js, module: glue.module(), memory };
  const workers: Worker[] = [];
  try {
    await Promise.all(
      Array.from({ length: threads }, () => new Promise<void>((resolve, reject) => {
        const worker = new Worker(new URL("./rust-pool.worker.ts", import.meta.url), { type: "module" });
        workers.push(worker);
        worker.onmessage = ({ data }: MessageEvent<string | null>) => (data === null ? resolve() : reject(new Error(data)));
        worker.onerror = (event) => reject(new Error(event.message || "a pool worker did not start"));
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

/** Feeds `unit` to `decoder` and reads the picture it decoded to, if any. */
export function decodeUnit(loaded: LoadedRust, decoder: RustDecoder, unit: Uint8Array): RustPicture | null {
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
  };
}

/** `ffmpeg -f framemd5`'s digest of the picture: its three planes, rows packed. */
export function rustPictureMd5(loaded: LoadedRust, picture: RustPicture): string {
  const heap = new Uint8Array(loaded.memory.buffer);
  const hasher = new Bun.CryptoHasher("md5");
  for (const plane of picture.planes) {
    for (let y = 0; y < picture.height; y++) {
      hasher.update(heap.subarray(plane.address + y * plane.stride, plane.address + y * plane.stride + picture.width));
    }
  }
  return hasher.digest("hex");
}
