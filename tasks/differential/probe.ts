// What each reader makes of one input, to decide a difference (AGENTS.md "Deciding a difference"):
// ours and the dialect's oracle (in the signature `run.ts` compares), the documented shape where they differ,
// the norm's errors, what browsers build (html), and what Prettier at the pin prints.
//
// Usage:
//   node tasks/differential/probe.ts <html|vue|angular|all> <source>
// `<source>` as a JSON string literal is unescaped (`"<a> </a>"`); `-` reads it from stdin.
// `all` runs every dialect (step 4's shared reading), the source in `<template>` for Vue unless it is a file already.
import fs from "node:fs";
import { parseTemplate } from "@angular/compiler";
import { compile as compileVue, parse as parseVue } from "@vue/compiler-dom";
import { parse as parseDocument, parseFragment } from "parse5";
import * as prettier from "prettier";
import { DIALECTS, matchShape, ours } from "./dialects.ts";

const [which, arg] = process.argv.slice(2);
if (!(which in DIALECTS || which === "all") || arg === undefined) {
  console.error(`usage: node tasks/differential/probe.ts <${Object.keys(DIALECTS).join("|")}|all> <source | "json" | ->`);
  process.exit(1);
}
const input = arg === "-" ? fs.readFileSync(0, "utf8") : arg.startsWith('"') ? JSON.parse(arg) : arg;
const show = (label: string, value: unknown) => console.log(`${label.padEnd(10)}${typeof value === "string" ? value : JSON.stringify(value)}`);
const isPage = (source: string) => /^\s*(?:<!doctype|<html)/i.test(source);

/** Vue's SFC parse, its errors collected rather than thrown. */
function vueSfc(source: string): { ast: any; errors: string[] } {
  const errors: string[] = [];
  const ast = parseVue(source, { parseMode: "sfc", comments: true, onError: (e: any) => errors.push(e.message) });
  return { ast, errors };
}

/** Vue's compile warnings, which are no errors. */
const warnings: string[] = [];

/** The norm's errors on the input (AGENTS.md "The one rule": what "without an error" is checked with). */
function normErrors(dialect: string, source: string): string[] {
  if (dialect === "html") {
    const errors: string[] = [];
    (isPage(source) ? parseDocument : parseFragment)(source, { onParseError: (e: any) => errors.push(e.code) });
    return errors;
  }
  if (dialect === "vue") {
    // The root `<template>`'s content, as Vue's SFC compiler hands it over; the whole input otherwise
    const root = vueSfc(source).ast.children.find((n: any) => n.type === 1 && n.tag === "template");
    const content = root?.children.length
      ? source.slice(root.children[0].loc.start.offset, root.children.at(-1).loc.end.offset)
      : root
        ? ""
        : source;
    const errors: string[] = [];
    // A warning is no error (AGENTS.md "The one rule"): shown apart
    const warn = console.warn;
    console.warn = (message: string) => warnings.push(String(message).split("\n")[0]);
    try {
      compileVue(content, { comments: true, onError: (e: any) => errors.push(e.message) });
    } finally {
      console.warn = warn;
    }
    return errors;
  }
  return parseTemplate(source, "template.html", { preserveWhitespaces: true }).errors?.map((e: any) => e.msg) ?? [];
}

/** What browsers build: parse5, as a fragment in `<body>` unless the input is a page, with scripting on. */
function browserTree(source: string): unknown {
  const tree = (nodes: any[]): unknown[] =>
    nodes.map((n) =>
      n.nodeName === "#text"
        ? ["t", n.value]
        : n.nodeName === "#comment"
          ? ["c", n.data]
          : n.nodeName === "#documentType"
            ? ["d", n.name]
            : ["el", n.tagName, n.attrs.map((a: any) => [a.name, a.value]), tree(n.content ? n.content.childNodes : n.childNodes)],
    );
  return tree((isPage(source) ? parseDocument(source) : parseFragment(source, { scriptingEnabled: true })).childNodes);
}

/** Vue's texts whose value is not their source: what Vue drops or decodes, which the signature (by source) does not show. */
function vueTexts(source: string): [string, string][] {
  const out: [string, string][] = [];
  const walk = (nodes: any[]) => {
    for (const n of nodes) {
      if (n.type === 2 && n.content !== n.loc.source) out.push([n.loc.source, n.content]);
      if (n.children) walk(n.children);
    }
  };
  walk(vueSfc(source).ast.children);
  return out;
}

async function probe(dialect: string, source: string) {
  const spec = DIALECTS[dialect];
  const [mine] = ours(dialect, [source]);
  const normalized = typeof mine === "string" ? mine : spec.normalize(structuredClone(mine));
  let oracle: unknown;
  try {
    oracle = spec.oracle(source);
  } catch (e) {
    oracle = `threw: ${e}`;
  }
  const same = JSON.stringify(normalized) === JSON.stringify(oracle);
  show("ours", normalized);
  if (JSON.stringify(mine) !== JSON.stringify(normalized)) show("ours raw", mine);
  show("oracle", same ? "same" : oracle);
  const [oursError, oracleError] = [typeof normalized === "string", typeof oracle === "string"];
  const shape = matchShape(spec.pending ?? {}, source) ?? matchShape({ ...spec.quirks, ...spec.mutationQuirks }, source);
  if (oursError && oracleError) show("shape", "both report an error");
  else if (oracleError) show("shape", "lenient: the oracle reports it, we parse");
  else if (shape) show("shape", `documented: ${shape} (a quirk in dialects.ts, or pending; its DIVERGENCES.md entry decides)`);
  else if (!same) show("shape", "none documented");
  if (dialect === "vue") {
    const texts = vueTexts(source);
    if (texts.length > 0) show("vue text", texts);
  }
  warnings.length = 0;
  const errors = normErrors(dialect, source);
  const label = dialect === "html" ? "parse5 errors (its tokenizer's and a few of its tree builder's: judge the others from the spec)" : "errors";
  show("norm", errors.length === 0 ? `no ${label}` : `${label}: ${errors.join(" | ")}`);
  if (warnings.length > 0) show("warnings", `${warnings.join(" | ")} (no errors)`);
  if (dialect === "html") {
    show("browser", browserTree(source));
    if (/<select\b/i.test(source)) show("note", "parse5 7.3.0 predates WHATWG's customizable <select>: for select, the spec is the norm");
  }
  try {
    show("prettier", "\n" + (await prettier.format(source, { parser: dialect, printWidth: 80 })));
  } catch (e: any) {
    show("prettier", `error: ${String(e.message).split("\n")[0]}`);
  }
}

if (which === "all") {
  for (const dialect of Object.keys(DIALECTS)) {
    const vueFile = /^\s*<(?:template|script|style)\b/i.test(input) || isPage(input);
    const source = dialect === "vue" && !vueFile ? `<template>${input}</template>` : input;
    console.log(`== ${dialect}${source === input ? "" : ` (in <template>)`}`);
    await probe(dialect, source);
  }
} else {
  await probe(which, input);
}
