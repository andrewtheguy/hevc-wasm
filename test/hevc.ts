// Loads build/out/hevc.js under Bun, as the page loads it in its worker, and
// reads `hevc_picture` the way remotex's hevcWasm.worker.ts does.
//
// The module is built for a worker environment: the glue fetches hevc.wasm next
// to itself and starts one pthread worker per slice thread before it resolves.
// Bun runs both, given the globals the glue looks for.

import { existsSync } from "node:fs";
import { resolve } from "node:path";

export const WASM_DIR = resolve(process.env.HEVC_WASM_DIR ?? `${import.meta.dir}/../build/out`);

export interface HevcModule {
  _hevc_create(threads: number): number;
  _hevc_input(decoder: number, size: number): number;
  _hevc_decode(decoder: number, keyframe: number): number;
  _hevc_picture(decoder: number): number;
  _hevc_destroy(decoder: number): void;
  wasmMemory: WebAssembly.Memory;
}

// 0 = 4:2:0, 1 = 4:2:2, 2 = 4:4:4, -1 = anything a VideoFrame cannot hold.
export type Format = -1 | 0 | 1 | 2;

export interface Plane {
  address: number;
  stride: number;
}

export interface Picture {
  width: number;
  height: number;
  format: Format;
  // FFmpeg's AVColorRange, AVColorSpace, AVColorPrimaries, AVColorTransferCharacteristic.
  range: number;
  colorspace: number;
  primaries: number;
  transfer: number;
  planes: [Plane, Plane, Plane];
  keyframe: boolean;
}

const pthreadShim = new URL("./pthread-shim.ts", import.meta.url).href;
let spawned: Worker[] | null = null;

function installGlobals() {
  const scope = globalThis as { WorkerGlobalScope?: unknown; Worker: typeof Worker };
  if (scope.WorkerGlobalScope) return;
  scope.WorkerGlobalScope = class WorkerGlobalScope {};
  const NativeWorker = scope.Worker;
  scope.Worker = class extends NativeWorker {
    constructor(url: string | URL, options: WorkerOptions = {}) {
      super(url, { ...options, preload: [pthreadShim] });
      spawned?.push(this);
    }
  };
}

export interface LoadedModule {
  module: HevcModule;
  // Terminates the module's pthread workers, which would otherwise keep the
  // process alive.
  close(): void;
}

// Instantiates the module with a pool of `threads` pthread workers. Loads must
// not overlap, since each one claims the workers started while it runs.
export async function loadHevc(threads: number): Promise<LoadedModule> {
  const glue = `${WASM_DIR}/hevc.js`;
  if (!existsSync(glue) || !existsSync(`${WASM_DIR}/hevc.wasm`)) {
    throw new Error(`no hevc.js and hevc.wasm in ${WASM_DIR}: run ./build.sh, or set HEVC_WASM_DIR`);
  }
  installGlobals();
  const { default: createHevcModule } = (await import(glue)) as {
    default: (options: { threads: number }) => Promise<HevcModule>;
  };
  const workers: Worker[] = [];
  spawned = workers;
  try {
    const module = await createHevcModule({ threads });
    return { module, close: () => workers.forEach((w) => w.terminate()) };
  } finally {
    spawned = null;
  }
}

export class Decoder {
  readonly #m: HevcModule;
  #d: number;

  constructor(module: HevcModule, threads: number) {
    this.#m = module;
    this.#d = module._hevc_create(threads);
    if (!this.#d) throw new Error(`hevc_create(${threads}) failed`);
  }

  get pointer(): number {
    return this.#d;
  }

  // The linear memory, viewed afresh since growth replaces its buffer.
  get heap(): Uint8Array {
    return new Uint8Array(this.#m.wasmMemory.buffer);
  }

  // hevc_decode's result: 1 with a picture, 0 without one, or a negative AVERROR.
  decode(unit: Uint8Array, keyframe = false): number {
    const input = this.#m._hevc_input(this.#d, unit.length) >>> 0;
    if (!input) throw new Error(`hevc_input(${unit.length}) failed`);
    this.heap.set(unit, input);
    return this.#m._hevc_decode(this.#d, keyframe ? 1 : 0);
  }

  picture(): Picture {
    const address = this.#m._hevc_picture(this.#d) >>> 0;
    const p = new Int32Array(this.#m.wasmMemory.buffer, address, 16);
    const plane = (i: number): Plane => ({ address: p[7 + i]! >>> 0, stride: p[10 + i]! });
    return {
      width: p[0]!,
      height: p[1]!,
      format: p[2] as Format,
      range: p[3]!,
      colorspace: p[4]!,
      primaries: p[5]!,
      transfer: p[6]!,
      planes: [plane(0), plane(1), plane(2)],
      keyframe: p[13] === 1,
    };
  }

  destroy() {
    this.#m._hevc_destroy(this.#d);
    this.#d = 0;
  }
}

// Each plane's visible rows, packed without stride padding: the frame
// `ffmpeg -f framemd5` hashes for rawvideo.
export function packedPlanes(heap: Uint8Array, picture: Picture): Uint8Array[] {
  const { width, height, format } = picture;
  if (format === -1) throw new Error("the picture is not 8-bit Y'CbCr");
  const cw = format === 2 ? width : Math.ceil(width / 2);
  const ch = format === 0 ? Math.ceil(height / 2) : height;
  return picture.planes.map(({ address, stride }, i) => {
    const [w, h] = i === 0 ? [width, height] : [cw, ch];
    const out = new Uint8Array(w * h);
    for (let y = 0; y < h; y++) {
      out.set(heap.subarray(address + y * stride, address + y * stride + w), y * w);
    }
    return out;
  });
}

export function pictureMd5(heap: Uint8Array, picture: Picture): string {
  const hash = new Bun.CryptoHasher("md5");
  for (const plane of packedPlanes(heap, picture)) hash.update(plane);
  return hash.digest("hex");
}
