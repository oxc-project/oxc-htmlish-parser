// What each reader makes of one input, to decide a difference (AGENTS.md "Deciding a difference"):
// ours and the dialect's oracle (in the signature `run.ts` compares), the documented shape it has,
// whether the norm reads it without an error, what browsers build (html), and what Prettier at the pin prints.
//
// Usage:
//   node tasks/differential/probe.ts <html|vue|angular> <source>
// `<source>` as a JSON string literal is unescaped (`"<a> </a>"`); `-` reads it from stdin.
import fs from "node:fs";
import { parseTemplate } from "@angular/compiler";
import { compile as compileVue, parse as parseVue } from "@vue/compiler-dom";
import { parse as parseDocument, parseFragment } from "parse5";
import * as prettier from "prettier";
import { DIALECTS, matchShape, ours } from "./dialects.ts";

const [dialect, arg] = process.argv.slice(2);
const spec = DIALECTS[dialect];
if (!spec || arg === undefined) {
  console.error(`usage: node tasks/differential/probe.ts <${Object.keys(DIALECTS).join("|")}> <source | "json" | ->`);
  process.exit(1);
}
const source = arg === "-" ? fs.readFileSync(0, "utf8") : arg.startsWith('"') ? JSON.parse(arg) : arg;
const show = (label: string, value: unknown) => console.log(`${label.padEnd(10)}${typeof value === "string" ? value : JSON.stringify(value)}`);

/** The norm's errors on the input (AGENTS.md "The one rule": what "without an error" is checked with). */
function normErrors(): string[] {
  if (dialect === "html") {
    const errors: string[] = [];
    const page = /^\s*(?:<!doctype|<html)/i.test(source);
    (page ? parseDocument : parseFragment)(source, { onParseError: (e: any) => errors.push(e.code) });
    return errors;
  }
  if (dialect === "vue") {
    // The root `<template>`'s content, as Vue's SFC compiler hands it over; the whole input otherwise
    const root = parseVue(source, { parseMode: "sfc" }).children.find((n: any) => n.type === 1 && n.tag === "template") as any;
    const content = root?.children.length
      ? source.slice(root.children[0].loc.start.offset, root.children.at(-1).loc.end.offset)
      : root ? "" : source;
    const errors: string[] = [];
    compileVue(content, { comments: true, onError: (e: any) => errors.push(e.message) });
    return errors;
  }
  return parseTemplate(source, "template.html", { preserveWhitespaces: true }).errors?.map((e: any) => e.msg) ?? [];
}

/** What browsers build: parse5, as a fragment in `<body>` unless the input is a page, with scripting on. */
function browserTree(): unknown {
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
  const page = /^\s*(?:<!doctype|<html)/i.test(source);
  return tree((page ? parseDocument(source) : parseFragment(source, { scriptingEnabled: true })).childNodes);
}

const [mine] = ours(dialect, [source]);
const oursSig = typeof mine === "string" ? mine : spec.normalize(mine);
let oracle: unknown;
try {
  oracle = spec.oracle(source);
} catch (e) {
  oracle = `threw: ${e}`;
}
show("ours", oursSig);
show("oracle", JSON.stringify(oursSig) === JSON.stringify(oracle) ? "same" : oracle);
const shape = matchShape(spec.pending ?? {}, source) ?? matchShape({ ...spec.quirks, ...spec.mutationQuirks }, source);
if (shape) show("shape", `${shape} (a run.ts quirk or pending construct; its DIVERGENCES.md entry decides)`);
const errors = normErrors();
show("norm", errors.length === 0 ? "reads it without an error" : `errors: ${errors.join(" | ")}`);
if (dialect === "html") show("browser", browserTree());
try {
  show("prettier", "\n" + (await prettier.format(source, { parser: dialect, printWidth: 80 })));
} catch (e: any) {
  show("prettier", `error: ${String(e.message).split("\n")[0]}`);
}
