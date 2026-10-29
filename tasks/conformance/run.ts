// Pinned real-file corpus, adapted from the sibling parsers' conformance runners.
// Compare against this repo's dialect oracles, not a browser's rebuilt HTML tree.
// Snapshots record existing gaps as well as documented quirks; every change needs review.
// Usage: node tasks/conformance/run.ts [html|vue|angular] [--check|--clone|--clean]
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { DIALECTS, matchShape, type Sig, UnknownNode } from "../differential/dialects.ts";
import { buildSignature, signature } from "../differential/harness.ts";

const DIR = import.meta.dirname;
const REPO = path.join(DIR, "repos/prettier");
const VERSION = "3.9.10"; // Same pin as tasks/differential/package.json.
const SHA = "1172b82c295c4f889fd787dc6c78c3e194ebf353";
const ALL_DIALECTS = ["html", "vue", "angular"];
const args = process.argv.slice(2);
const modes = args.filter((arg) => arg.startsWith("--"));
const dialects = args.filter((arg) => !arg.startsWith("--"));
if (modes.length > 1 || modes.some((arg) => !["--check", "--clone", "--clean"].includes(arg)) ||
    dialects.some((dialect) => !ALL_DIALECTS.includes(dialect))) {
  throw new Error("usage: conformance/run.ts [html|vue|angular] [--check|--clone|--clean]");
}
if (args.includes("--clean")) {
  fs.rmSync(path.join(DIR, "repos"), { recursive: true, force: true });
  process.exit(0);
}
const selected = dialects.length > 0 ? [...new Set(dialects)] : ALL_DIALECTS;
const pkg = JSON.parse(fs.readFileSync(path.join(DIR, "../differential/package.json"), "utf8"));
if (pkg.dependencies.prettier !== VERSION) throw new Error("Bump the conformance corpus together with the Prettier oracle pin");

function git(...gitArgs: string[]): string {
  const r = spawnSync("git", ["-C", REPO, ...gitArgs], { encoding: "utf8", maxBuffer: 1 << 24 });
  if (r.error) throw r.error;
  if (r.status !== 0) throw new Error(`git ${gitArgs.join(" ")} failed:\n${r.stderr}`);
  return r.stdout;
}

fs.mkdirSync(REPO, { recursive: true });
if (!fs.existsSync(path.join(REPO, ".git"))) {
  git("init", "-q");
  git("remote", "add", "origin", "https://github.com/prettier/prettier.git");
}
const head = spawnSync("git", ["-C", REPO, "rev-parse", "HEAD"], { encoding: "utf8" });
if (head.stdout.trim() !== SHA) {
  git("sparse-checkout", "set", ...ALL_DIALECTS.map((dialect) => `tests/format/${dialect}`));
  git("fetch", "--depth", "1", "--filter=blob:none", "origin", SHA);
  git("checkout", "--detach", "FETCH_HEAD");
}
if (git("status", "--porcelain").trim() !== "") {
  throw new Error("The cached Prettier corpus has local changes; run `just conformance-clean` to refetch it");
}
if (args.includes("--clone")) {
  console.log(`Prettier ${VERSION} corpus ready (${SHA})`);
  process.exit(0);
}

function classify(dialect: string, source: string, raw: Sig): string {
  const spec = DIALECTS[dialect];
  const ours = typeof raw === "string" ? raw : spec.normalize(raw);
  if (typeof ours === "string" && ours.startsWith("invariant: ")) throw new Error(ours);
  let oracle: Sig;
  try {
    oracle = spec.oracle(source);
  } catch (error) {
    if (error instanceof UnknownNode) throw error;
    oracle = `error: oracle crashed: ${error}`;
  }
  const oursError = typeof ours === "string";
  const oracleError = typeof oracle === "string";
  if (oursError && oracleError) return "bothError";
  if (oracleError) return "lenient";
  const kind = oursError ? "oursError" : JSON.stringify(ours) === JSON.stringify(oracle) ? "same" : "mismatch";
  if (kind === "same") return kind;
  const pending = matchShape(spec.pending ?? {}, source);
  if (pending) return `pending: ${pending}`;
  const quirk = matchShape({ ...spec.quirks, ...spec.mutationQuirks }, source);
  return quirk ? `quirk: ${quirk}` : kind;
}

buildSignature();
let changed = false;
for (const dialect of selected) {
  const prefix = `tests/format/${dialect}/`;
  const files = git("ls-files", "-z", prefix).split("\0")
    .filter((file) => /\.(?:html|vue)$/.test(file)).sort();
  if (files.length === 0) throw new Error(`No ${dialect} fixtures were found`);
  const tally = new Map<string, number>();
  const gaps: string[] = [];
  for (let at = 0; at < files.length; at += 64) {
    const batch = files.slice(at, at + 64);
    const inputs = batch.map((file) => fs.readFileSync(path.join(REPO, file), "utf8"));
    const outputs = signature(dialect, inputs);
    batch.forEach((file, index) => {
      let kind: string;
      try {
        kind = classify(dialect, inputs[index], outputs[index]);
      } catch (error) {
        throw new Error(`${file}: ${error}`);
      }
      tally.set(kind, (tally.get(kind) ?? 0) + 1);
      if (kind !== "same" && kind !== "bothError") gaps.push(`${kind}: ${file.slice(prefix.length)}`);
    });
  }
  const summary = [...tally].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
    .map(([kind, count]) => `  ${kind}: ${count}`).join("\n");
  const snapshot = `Prettier ${VERSION} (${SHA})\n${dialect}: ${files.length} files\n${summary}\n\n${gaps.join("\n")}\n`;
  const file = path.join(DIR, "snapshots", `${dialect}.snap`);
  if (args.includes("--check")) {
    const old = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : undefined;
    if (old !== snapshot) {
      console.error(`${dialect}: corpus results changed; run \`just conformance ${dialect}\`, review and commit the snapshot`);
      changed = true;
    }
  } else {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, snapshot);
  }
  console.log(`${dialect}: ${files.length} files\n${summary}`);
}
if (changed) process.exit(1);
