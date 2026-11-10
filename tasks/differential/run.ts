// Differential fuzz against each dialect's parse-behavior oracle.
//
// - html: angular-html-parser, with the options Prettier passes
// - vue: @vue/compiler-dom in SFC mode
// - angular: @angular/compiler, with the options its template compiler parses with
//
// Reduces each input to a structural signature on both sides (ours from `tasks/sig`'s `sig-batch`),
// and reports where they differ. Failures are a tree mismatch, an error from us on input the oracle accepts,
// an AST invariant we break (`tests/check`), or an oracle throwing (a node kind the signature does not know);
// the oracle's error where we parse is counted as lenient, as DIVERGENCES.md allows.
//
// Inputs are a deterministic corpus of markup-token soups, half of them mutated byte-wise
// (an inserted, deleted or replaced `<`, `>`, `/`, quote, ...) so that broken tag internals come up too,
// or real files with `--files`.
// Whole-tag tokens never hold the shapes DIVERGENCES.md documents (`/>` on non-void HTML elements,
// unquoted values with `/`, `foreignObject`); a mutation may form one, which `mutationQuirks` then exempts.
//
// Usage:
//   node tasks/differential/run.ts <html|vue|angular> [count] [seed]
//   node tasks/differential/run.ts <html|vue|angular> --files <file>...
import fs from "node:fs";
import { DIALECTS, matchShape, ours, type Sig, UnknownNode, VOID } from "./dialects.ts";
import { Rng } from "./harness.ts";

// ---
// Inputs

/**
 * A soup where `CLOSE` closes the latest open tag (or block) of the soup itself,
 * so most inputs nest instead of failing on stray end tags.
 */
function soup(rng: Rng, tokens: readonly string[], n: number): string {
  const open: string[] = [];
  let out = "";
  const close = (name: string) => (name === "}" ? "}" : `</${name}>`);
  for (let i = 0; i < n; i++) {
    const token = tokens[rng.int(0, tokens.length - 1)];
    if (token === "CLOSE") {
      const name = open.pop();
      if (name) out += close(name);
      continue;
    }
    const name = /^<([a-zA-Z][^\s/>]*)[^<]*?(?<!\/)>$/.exec(token)?.[1];
    if (name && !VOID.has(name.toLowerCase())) open.push(name);
    if (token.endsWith("{") && token.startsWith("@")) open.push("}");
    out += token;
  }
  while (open.length > 0 && rng.random() < 0.9) out += close(open.pop()!);
  return out;
}

const MUTATIONS = ["<", ">", "/", "-", "!", "=", '"', "'", " ", "\n", "{", "}", "@"];

/** One to three byte edits: insert, delete or replace a character markup is made of. */
function mutate(rng: Rng, source: string): string {
  let out = source;
  for (let k = rng.int(1, 3); k > 0; k--) {
    const at = rng.int(0, out.length);
    const c = MUTATIONS[rng.int(0, MUTATIONS.length - 1)];
    const rest = rng.int(0, 2) === 0 ? out.slice(at) : out.slice(at + 1);
    out = out.slice(0, at) + (rng.int(0, 2) === 0 ? "" : c) + rest;
  }
  return out;
}

/** The two signatures around where they first differ. */
function firstDiff(a: string, b: string): string {
  let i = 0;
  while (i < a.length && a[i] === b[i]) i++;
  const around = (s: string) => s.slice(Math.max(0, i - 60), i + 60);
  return `ours:   …${around(a)}…\noracle: …${around(b)}…`;
}

const [dialect, ...rest] = process.argv.slice(2);
const spec = DIALECTS[dialect];
if (!spec) {
  console.error(`usage: node tasks/differential/run.ts <${Object.keys(DIALECTS).join("|")}> [count] [seed] | --files <file>...`);
  process.exit(1);
}

let corpus: string[];
/** A file name, or the generated input's index (its source printed with a failure). */
let label: (i: number) => string;
/** Whether every quirk applies: mutated inputs and files are not limited to whole-tag tokens. */
let loose: boolean[];
let header: string;
if (rest[0] === "--files") {
  const files = rest.slice(1);
  corpus = files.map((file) => fs.readFileSync(file, "utf8"));
  label = (i) => files[i];
  loose = corpus.map(() => true);
  header = `${dialect} files=${files.length}`;
} else {
  const count = Number(rest[0] ?? 5000);
  const seed = Number(rest[1] ?? 0);
  const rng = new Rng(seed);
  const tokens = [...spec.tokens, ...Array(15).fill("CLOSE")];
  loose = [];
  corpus = Array.from({ length: count }, (_, i) => {
    const body = soup(rng, tokens, rng.int(1, 30));
    loose[i] = rng.random() < 0.5;
    return spec.wrap(loose[i] ? mutate(rng, body) : body);
  });
  label = (i) => `#${i} ${JSON.stringify(corpus[i])}`;
  header = `${dialect} count=${count} seed=${seed}`;
}

const theirs = corpus.map((source): Sig => {
  try {
    return spec.oracle(source);
  } catch (e) {
    // The oracle crashing on its own (angular-html-parser on an unterminated CDATA) fails Prettier too.
    return e instanceof UnknownNode ? `threw: ${e.message}` : `error: oracle crashed: ${e}`;
  }
});
const mine = ours(dialect, corpus).map((sig) => (typeof sig === "string" ? sig : spec.normalize(sig)));

const looseQuirks = { ...spec.quirks, ...spec.mutationQuirks };

const tally: Record<string, number> = { same: 0, bothError: 0, lenient: 0, oursError: 0, mismatch: 0 };
const failures: string[] = [];
corpus.forEach((source, i) => {
  const [a, b] = [mine[i], theirs[i]];
  const aError = typeof a === "string";
  const bError = typeof b === "string";
  let kind: string;
  if (aError && a.startsWith("invariant: ")) kind = "invariant";
  else if (bError && b.startsWith("threw: ")) kind = "oracleThrew";
  else if (aError && bError) kind = "bothError";
  else if (bError) kind = "lenient";
  else if (aError) kind = "oursError";
  else kind = JSON.stringify(a) === JSON.stringify(b) ? "same" : "mismatch";
  if (kind === "oursError" || kind === "mismatch") {
    const pending = matchShape(spec.pending ?? {}, source);
    const quirk = process.env.NO_QUIRKS ? undefined : matchShape(loose[i] ? looseQuirks : spec.quirks, source);
    if (pending) kind = `pending: ${pending}`;
    else if (quirk) kind = `quirk: ${quirk}`;
  }
  if (["oursError", "mismatch", "invariant", "oracleThrew"].includes(kind)) {
    const detail = kind === "mismatch" ? firstDiff(JSON.stringify(a), JSON.stringify(b)) : `ours:   ${JSON.stringify(a)}\noracle: ${JSON.stringify(b)}`;
    failures.push(`--- ${kind} ${label(i)}\n${detail}`);
  }
  tally[kind] = (tally[kind] ?? 0) + 1;
});

// `ALL=1` prints every failure, not the first ten.
console.log((process.env.ALL ? failures : failures.slice(0, 10)).join("\n"));
if (process.env.ERRORS) {
  const messages: Record<string, number> = {};
  for (const sig of [...mine, ...theirs]) {
    if (typeof sig !== "string") continue;
    const key = sig.replace(/ at \d+.*| ".*/, "");
    messages[key] = (messages[key] ?? 0) + 1;
  }
  console.log(messages);
}
console.log(header, tally);
process.exit(failures.length > 0 ? 1 : 0);
