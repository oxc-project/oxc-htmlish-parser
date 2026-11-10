// Panic-safety fuzz, adapted from oxc-markdown-parser: valid UTF-8 may error, but must not crash
// or break the AST invariants. Usage: node tasks/differential/panic.ts <dialect> [count] [seed]
import { buildSignature, Rng, signature } from "./harness.ts";

const [dialect, countArg = "10000", seedArg = "0", ...extra] = process.argv.slice(2);
const count = Number(countArg);
const seed = Number(seedArg);
if (!["html", "vue", "angular"].includes(dialect) || extra.length > 0 ||
    !Number.isSafeInteger(count) || count < 1 || !Number.isSafeInteger(seed)) {
  throw new Error("usage: panic.ts <html|vue|angular> [positive count] [integer seed]");
}

const CHARS = [
  "<", ">", "/", "!", "?", "-", "=", '"', "'", "&", ";", "[", "]", "{", "}",
  "(", ")", "@", ":", ".", "#", "*", "\\", "`", "$", "a", "1",
  "é", "ß", "漢", "字", "（", "）", "🎉", "𝄞", "𓀀",
  "\u0301", "\u200d", "\u200b", "\ufeff", "\ufffd", "\uffff",
  " ", "\t", "\n", "\r", "\f", "\u00a0", "\u2028", "\u0085", "\0", "\x01", "\x7f",
];

const TEMPLATES = [
  (x: string) => `<a x='${x}'>${x}</a>`,
  (x: string) => `<${x} ${x}="${x}">`,
  (x: string) => `<!--${x}-->`,
  (x: string) => `<![CDATA[${x}]]>`,
  (x: string) => `<script>${x}</script><style>${x}</style>`,
  (x: string) => `<textarea>\r\n${x}</textarea>`,
  (x: string) => `<svg><foreignObject><div>${x}</div></foreignObject></svg>`,
  (x: string) => `{{${x}}}`,
  ...(dialect === "vue" ? [
    (x: string) => `<template><C :[${x}].prop="${x}" @click.stop="${x}" /></template>`,
    (x: string) => `<template><div v-pre>${x}</div></template>`,
  ] : []),
  ...(dialect === "angular" ? [
    (x: string) => `@if (${x}) {<p>${x}</p>} @else {${x}}`,
    (x: string) => `<div [${x}]="${x}" (${x})="${x}">${x}</div>`,
  ] : []),
];

function failure(inputs: string[]): string | undefined {
  try {
    const broken = signature(dialect, inputs).find((sig) => typeof sig === "string" && sig.startsWith("invariant: "));
    return typeof broken === "string" ? broken : undefined;
  } catch (error) {
    return String(error);
  }
}

buildSignature();
const rng = new Rng(seed);
for (let at = 0; at < count; at += 500) {
  const batch = Array.from({ length: Math.min(500, count - at) }, () => {
    const text = Array.from({ length: rng.int(1, 120) }, () => rng.choice(CHARS)).join("");
    return rng.random() < 0.5 ? rng.choice(TEMPLATES)(text) : text;
  });
  let error = failure(batch);
  if (!error) continue;
  let failing = batch;
  while (failing.length > 1) {
    const halves = [failing.slice(0, failing.length >> 1), failing.slice(failing.length >> 1)];
    const errors = halves.map(failure);
    const index = errors.findIndex((e) => e !== undefined);
    if (index === -1) throw new Error(`batch failure did not reproduce in either half:\n${error}`);
    failing = halves[index];
    error = errors[index];
  }
  console.error(`panic fuzz failed (${dialect}, seed ${seed}):\n${JSON.stringify(failing[0])}\n${error}`);
  process.exit(1);
}
console.log(`panic fuzz: ${dialect}, ${count} inputs survived (seed ${seed})`);
