// One thread of the pure-Rust decoder's pool under Bun: an instance of the module
// on the memory the test's instance has, which takes a seat in the pool and
// decodes the rows the pool hands it. Answers once, before it takes its seat.

import type { PoolSeat } from "./rust";

declare const self: Worker;

self.onmessage = async ({ data }: MessageEvent<PoolSeat>) => {
  try {
    const glue = await import(`${process.env.HEVC_RUST_DIR ?? `${import.meta.dir}/../rust/hevc-web/pkg`}/hevc_web.js`);
    await glue.default({ module_or_path: data.module, memory: data.memory });
    self.postMessage(null);
    glue.runPoolThread();
  } catch (error) {
    self.postMessage(error instanceof Error ? error.message : String(error));
  }
};
