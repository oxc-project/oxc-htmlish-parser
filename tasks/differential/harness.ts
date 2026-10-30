// Process and random-number helpers shared by the differential, corpus and panic runners.
import { spawnSync } from "node:child_process";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "../..");
const BIN = path.join(ROOT, "target/debug", process.platform === "win32" ? "sig-batch.exe" : "sig-batch");

/** mulberry32: deterministic inputs make failures reproducible. */
export class Rng {
  #state: number;
  constructor(seed: number) {
    this.#state = seed >>> 0;
  }
  random(): number {
    let t = (this.#state = (this.#state + 0x6d2b79f5) | 0);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  int(lo: number, hi: number): number {
    return lo + Math.floor(this.random() * (hi - lo + 1));
  }
  choice<T>(values: readonly T[]): T {
    return values[this.int(0, values.length - 1)];
  }
}

export function buildSignature(): void {
  const r = spawnSync("cargo", ["build", "-q", "-p", "sig"], { cwd: ROOT, encoding: "utf8" });
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(r.stderr);
}

export function signature(dialect: string, inputs: string[]): Array<unknown[] | string> {
  const r = spawnSync(BIN, [dialect], {
    input: JSON.stringify(inputs), encoding: "utf8", maxBuffer: 1 << 28,
  });
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(`sig-batch failed (a panic?):\n${r.stderr}`);
  const outputs = JSON.parse(r.stdout);
  if (!Array.isArray(outputs) || outputs.length !== inputs.length) {
    throw new Error(`sig-batch returned incomplete output for ${inputs.length} inputs`);
  }
  return outputs;
}
