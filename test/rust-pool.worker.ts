// One thread of the decoder's pool under Node or Bun: an instance of the module
// on the memory the caller's instance has, which takes a seat in the pool and
// decodes the rows the pool hands it. Answers once, before it takes its seat.

import { pathToFileURL } from "node:url";
import { parentPort } from "node:worker_threads";
import type { PoolSeat } from "./rust.ts";

const port = parentPort!;
port.once("message", async (data: PoolSeat) => {
  try {
    const glue = await import(pathToFileURL(data.js).href);
    await glue.default({ module_or_path: data.module, memory: data.memory });
    port.postMessage(null);
    glue.runPoolThread();
  } catch (error) {
    port.postMessage(error instanceof Error ? error.message : String(error));
  }
});
