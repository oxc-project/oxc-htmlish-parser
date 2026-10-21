// Compare our parser against itself at another revision, over real files.
//
// Parses every file with the `sig-batch` of `<rev>` (built once into a temp directory) and of the working tree,
// and reports what changed: a file that parsed and now fails, an AST invariant broken, a changed tree,
// or a file that failed and now parses. Run it before committing a change to how the parser reads a shape,
// on corpora the shape may hit (local repos, ecosystem checkouts); the oracle comparison is `run.ts`.
//
// Usage:
//   node tasks/differential/compare.ts <html|vue|angular> <rev> <file>...
//   node tasks/differential/compare.ts <html|vue|angular> <rev> --list <file listing paths, one per line>
// `<rev>`'s `sig-batch` must know the dialect. Exits 1 when a file now fails or breaks an invariant.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "../..");
const [dialect, rev, ...rest] = process.argv.slice(2);
if (!dialect || !rev || rest.length === 0) {
  console.error("usage: node tasks/differential/compare.ts <html|vue|angular> <rev> <file>... | --list <file>");
  process.exit(1);
}

function sh(cmd: string, args: string[], cwd: string, input?: string): string {
  const r = spawnSync(cmd, args, { cwd, input, encoding: "utf8", maxBuffer: 1 << 30 });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed:\n${r.stderr}`);
  return r.stdout;
}

/** `<rev>`'s `sig-batch`, built in its own checkout under the temp directory and reused. */
function baseline(): string {
  const sha = sh("git", ["rev-parse", "--short", rev], ROOT).trim();
  const dir = path.join(os.tmpdir(), `oxc-htmlish-parser-${sha}`);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir);
    sh("sh", ["-c", `git archive ${sha} | tar -x -C ${dir}`], ROOT);
  }
  sh("cargo", ["build", "-q", "-p", "sig"], dir);
  return path.join(dir, "target/debug/sig-batch");
}
sh("cargo", ["build", "-q", "-p", "sig"], ROOT);
const bins = [baseline(), path.join(ROOT, "target/debug/sig-batch")];

const listed = rest[0] === "--list" ? fs.readFileSync(rest[1], "utf8").split("\n").filter(Boolean) : rest;
// Generated or minified giants only slow the debug builds down.
const files = listed.filter((file) => fs.existsSync(file) && fs.statSync(file).size < 2_000_000);

/** Bogus comments are `bc` only in newer signatures. */
const normalize = (sig: unknown): unknown =>
  Array.isArray(sig) ? (sig[0] === "bc" ? ["c", sig[1]] : sig.map(normalize)) : sig;

const tally: Record<string, number> = {};
const changes: string[] = [];
for (let i = 0; i < files.length; i += 300) {
  const chunk = files.slice(i, i + 300);
  const input = JSON.stringify(chunk.map((file) => fs.readFileSync(file, "utf8")));
  const [before, after] = bins.map((bin) => (JSON.parse(sh(bin, [dialect], ROOT, input)) as unknown[]).map(normalize));
  chunk.forEach((file, j) => {
    const [a, b] = [JSON.stringify(before[j]), JSON.stringify(after[j])];
    const [aError, bError] = [typeof before[j] === "string", typeof after[j] === "string"];
    let kind: string;
    if (bError && (after[j] as string).startsWith("invariant: ")) kind = "invariant";
    else if (aError && bError) kind = "both fail";
    else if (bError) kind = "now fails";
    else if (aError) kind = "now parses";
    else kind = a === b ? "same" : "tree changed";
    tally[kind] = (tally[kind] ?? 0) + 1;
    if (kind === "same" || kind === "both fail") return;
    let k = 0;
    while (k < a.length && a[k] === b[k]) k++;
    const around = (s: string) => s.slice(Math.max(0, k - 80), k + 80);
    changes.push(`--- ${kind} ${file}\n  before: …${around(a)}…\n  after:  …${around(b)}…`);
  });
}

console.log(changes.join("\n"));
console.log(`${dialect} ${rev} files=${files.length}`, tally);
process.exit(tally["now fails"] || tally.invariant ? 1 : 0);
