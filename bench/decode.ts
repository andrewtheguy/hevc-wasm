// bun bench/decode.ts FILE.h265 THREADS [PICTURES]: the module in HEVC_WASM_DIR (build/out) decoding a stream, timed per
// access unit by its own clock. With CHECK=1 each picture's MD5 is compared with FILE.h265.framemd5, outside the timing.
import { accessUnits } from "../test/annexb";
import { decodeUnit, loadRust, rustPictureMd5 } from "../test/rust";

const [file, t = "1", limit = "100000"] = process.argv.slice(2);
if (!file) throw new Error("usage: bun bench/decode.ts FILE.h265 THREADS [PICTURES]");
const threads = Number(t);
const check = process.env.CHECK === "1";
const loaded = await loadRust(Math.max(threads, 1));
const units = accessUnits(new Uint8Array(await Bun.file(file).arrayBuffer())).slice(0, Number(limit));
const md5s = check ? (await Bun.file(`${file}.framemd5`).text()).split("\n").filter((l) => l && !l.startsWith("#")).map((l) => l.split(",").pop()!.trim()) : [];
const decoder = new loaded.glue.Decoder(threads);
const times: number[] = [];
let pictures = 0;
let mismatch = -1;
for (const unit of units) {
  const started = performance.now();
  const picture = decodeUnit(loaded, decoder, unit.data);
  times.push(performance.now() - started);
  if (!picture) continue;
  if (check && mismatch < 0 && rustPictureMd5(loaded, picture) !== md5s[pictures]) mismatch = pictures;
  pictures++;
}
const total = times.reduce((a, b) => a + b, 0);
times.sort((a, b) => a - b);
const at = (x: number) => times[Math.floor((times.length - 1) * x)]!.toFixed(2);
console.log(`${pictures} pictures, ${threads} threads: mean ${(total / pictures).toFixed(2)} ms, median ${at(0.5)}, p95 ${at(0.95)}, max ${at(1)}${check ? `, first mismatch ${mismatch < 0 ? "none" : mismatch}` : ""}`);
decoder.free();
loaded.close();
if (mismatch >= 0) process.exit(1);
