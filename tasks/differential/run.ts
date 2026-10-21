// Differential fuzz against each dialect's parse-behavior oracle.
//
// - html: angular-html-parser, with the options Prettier passes
// - vue: @vue/compiler-dom in SFC mode
//
// Generates a deterministic corpus of markup-token soups, reduces each to a
// structural signature on both sides (ours from `tasks/sig`'s `sig-batch`), and
// reports where they differ. Failures are a tree mismatch, or an error from us
// on input the oracle accepts; the reverse (the oracle rejects, we parse) is
// counted as lenient, as DIVERGENCES.md allows.
//
// Tokens are whole tags on purpose: shapes documented in DIVERGENCES.md (`/>` on
// non-void HTML elements, unquoted values with `/`, `foreignObject`) are never
// generated, the fixtures pin them instead.
//
// Usage: node tasks/differential/run.ts <html|vue> [count] [seed]
import { spawnSync } from "node:child_process";
import path from "node:path";
import { parse as parseVue } from "@vue/compiler-dom";
import { parseHtml } from "angular-html-parser";

const ROOT = path.resolve(import.meta.dirname, "../..");
const BIN = path.join(ROOT, "target/debug/sig-batch");

type Sig = unknown[] | string;

/** mulberry32: `Math.random` is unseedable, and the corpus must be reproducible. */
class Rng {
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
}

const TEXT = ["a", "foo bar", " ", "  ", "\n", "\r\n", "\t", "é", "<", ">", "&amp;", "&", '"', "'", "=", "/", "{", "}"];

const HTML_TOKENS = [
  ...TEXT,
  "<div>", "<DIV>", "<span>", "<b>", "<A>", "</a>", "<my-el>",
  "<p>", "</p>", "<ul>", "<li>", "</li>", "<dl>", "<dt>", "<dd>",
  "<table>", "<tr>", "<td>", "</td>", "<th>", "<tbody>", "<thead>",
  "<select>", "<option>", "<optgroup>", "</option>",
  "<br>", "<br/>", "<br />", "<img src=x>", "<hr>", "</br>", "<input value='1'>",
  "<div a=1 b=\"x y\" c='z' d>", "<div\n  a\n>", "</ div >", "<p\tclass=\"c\">",
  "<pre>", "<pre>\n", "<pre>\r\n", "<textarea>\nx &amp; <b></textarea>", "<title>t</title>",
  "<script>a < b</script>", "<script>x</SCRIPT >", "<style>a>b{}</style>",
  "<svg>", "<svg><path d=\"M0\"/></svg>", "<svg><title><b>t</title></svg>", "<math>",
  "<!-- c -->", "<!x>", "<?x?>", "<![CDATA[x]]>", "<!doctype html>", "<!DOCTYPE html>",
  "<!--[if IE]><p>ie</p><![endif]-->", "<!--[if !IE]><!-->", "<!--<![endif]-->",
  "{{ a }}", "{{", "}}",
];

const VUE_TOKENS = [
  ...TEXT,
  "<div>", "<span>", "<p>", "<li>", "<C>", "</c>", "<Transition>", "</transition>",
  "<div/>", "<C />", "<br>", "<br/>", "<input>", "<Input>",
  "<div :a=\"b\" @click.stop=\"f\" v-if=\"x\">", "<C #default=\"{ i }\" #item.name v-slot:h>",
  "<C v-bind:[k].camel=\"v\" .p=\"v\" @[e]=\"h\" v-on:keyup.enter.prevent=\"g\">",
  "<div v-pre>", "<div a=b c>", "<div v-model.trim='m'>",
  "<pre>", "<pre>\n", "<textarea>\n{{ t }}</textarea>", "<title>{{ t }} <b></title>",
  "<script>a < b</script>", "<style>a>b{}</style>", "<svg>", "<svg><style>a<b</style></svg>",
  "<!-- c -->", "{{ a }}", "{{ a<b }}", "{{ '}}' }}", "{{", "}}", "{{ <div> }}",
];

// ---
// Oracle signatures, the shape `tasks/sig/src/main.rs` documents.

function stripQuotes(value: string): string {
  return /^(["']).*\1$/s.test(value) ? value.slice(1, -1) : value;
}

const LEADING_LF = /^(\r\n|\n|\r)/;
const IGNORES_FIRST_LF = new Set(["pre", "textarea", "listing"]);
const VOID = new Set("area,base,br,col,embed,hr,img,input,link,meta,param,source,track,wbr".split(","));
const VOID_TAG = `<(?:${[...VOID].join("|")})\\b[^<>]*(?<!/)>`;

/** The line break `pre` / `textarea` / `listing` drop: strip it from the first text. */
function leadingLf(children: unknown[][]): boolean {
  const first = children[0];
  if (first?.[0] !== "t") return false;
  const text = first[1] as string;
  const lf = LEADING_LF.exec(text);
  if (!lf) return false;
  const rest = text.slice(lf[0].length);
  if (rest === "") children.shift();
  else first[1] = rest;
  return true;
}

function htmlOracle(source: string): Sig {
  const { rootNodes, errors } = parseHtml(source, { canSelfClose: true, allowHtmComponentClosingTags: true });
  if (errors.length > 0) return `error: ${errors[0].msg}`;
  const slice = (span: any) => source.slice(span.start.offset, span.end.offset);
  const nodes = (nodes: any[]): unknown[][] => nodes.map(node);
  // angular-html-parser drops a text that was only the ignored line break;
  // its start tag's end tells whether there was one.
  const lfAfter = (el: any) => LEADING_LF.test(source.slice(el.startSourceSpan.end.offset));
  function node(n: any): unknown[] {
    switch (n.kind) {
      case "element": {
        const name = slice(n.nameSpan);
        const children = nodes(n.children);
        const prefix = name.includes(":") ? "" : n.name.match(/^:(\w+):/)?.[1];
        const lfTag = !prefix && !n.isSelfClosing && IGNORES_FIRST_LF.has(name.toLowerCase());
        const lf = lfTag && (leadingLf(children) || lfAfter(n));
        let flags = n.isSelfClosing ? "/" : "";
        if (n.endSourceSpan !== null && !n.isSelfClosing) flags += "e";
        if (lf) flags += "lf";
        const attrs = n.attrs.map((a: any) => [slice(a.keySpan), a.valueSpan ? stripQuotes(slice(a.valueSpan)) : null]);
        return ["el", name, flags, attrs, children];
      }
      case "text":
        return ["t", slice(n.sourceSpan)];
      case "comment": {
        const raw = slice(n.sourceSpan);
        return ["c", raw.startsWith("<!--") ? raw.slice(4, -3) : raw.slice(raw.startsWith("<!") ? 2 : 1, -1)];
      }
      case "docType":
        return ["d", slice(n.sourceSpan)];
      case "cdata":
        return ["x", slice(n.sourceSpan)];
      default:
        throw new Error(`unexpected node ${n.kind}`);
    }
  }
  return mergeTexts(nodes(rootNodes));
}

/** angular-html-parser splits escapable raw text at entities; a text is a text. */
function mergeTexts(nodes: unknown[][]): unknown[][] {
  const out: unknown[][] = [];
  for (const n of nodes) {
    const last = out.at(-1);
    if (n[0] === "t" && last?.[0] === "t") last[1] = (last[1] as string) + (n[1] as string);
    else out.push(n);
    if (n[0] === "el") n[4] = mergeTexts(n[4] as unknown[][]);
  }
  return out;
}


function vueOracle(source: string): Sig {
  const errors: any[] = [];
  const ast = parseVue(source, {
    parseMode: "sfc",
    comments: true,
    whitespace: "preserve",
    onError: (e: any) => errors.push(e),
  });
  if (errors.length > 0) return `error: ${errors[0].message}`;
  const nodes = (nodes: any[]): unknown[][] => nodes.map(node);
  function value(p: any, nameLength: number): string | null {
    const rest = p.loc.source.slice(nameLength);
    const eq = /^\s*=\s*/.exec(rest);
    return eq ? stripQuotes(rest.slice(eq[0].length)) : null;
  }
  function node(n: any): unknown[] {
    switch (n.type) {
      case 1: {
        const children = nodes(n.children);
        const lf = n.ns === 0 && (n.tag === "pre" || n.tag === "textarea") && leadingLf(children);
        let flags = n.isSelfClosing ? "/" : "";
        if (!n.isSelfClosing && !VOID.has(n.tag)) flags += "e";
        if (lf) flags += "lf";
        const attrs = n.props.map((p: any) => {
          // `v-pre` itself stays a directive in Vue; we keep everything under `v-pre` plain.
          if (p.type === 6 || p.name === "pre") {
            const name = p.type === 6 ? p.nameLoc.source : p.rawName;
            return [name, value(p, name.length)];
          }
          const arg = p.arg ? p.arg.loc.source : null;
          const modifiers = p.modifiers.map((m: any) => m.content);
          return [p.rawName, value(p, p.rawName.length), p.name, arg, p.arg ? !p.arg.isStatic : false, modifiers];
        });
        return ["el", n.tag, flags, attrs, children];
      }
      case 2:
        return ["t", n.loc.source];
      case 3:
        return ["c", n.content];
      case 5:
        return ["i", n.loc.source];
      default:
        throw new Error(`unexpected node type ${n.type}`);
    }
  }
  return nodes(ast.children);
}

// ---

function ours(dialect: string, inputs: string[]): Sig[] {
  const build = spawnSync("cargo", ["build", "-q", "-p", "sig"], { cwd: ROOT, encoding: "utf8" });
  if (build.status !== 0) throw new Error(build.stderr);
  const r = spawnSync(BIN, [dialect], { input: JSON.stringify(inputs), encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`sig-batch failed (a panic?):\n${r.stderr}`);
  return JSON.parse(r.stdout);
}

const [dialect, countArg, seedArg] = process.argv.slice(2);
if (dialect !== "html" && dialect !== "vue") {
  console.error("usage: node tasks/differential/run.ts <html|vue> [count] [seed]");
  process.exit(1);
}
const count = Number(countArg ?? 5000);
const seed = Number(seedArg ?? 0);
const rng = new Rng(seed);
/**
 * A soup where `CLOSE` closes the latest open tag of the soup itself,
 * so most inputs nest instead of failing on stray end tags.
 */
function soup(tokens: readonly string[], n: number): string {
  const open: string[] = [];
  let out = "";
  for (let i = 0; i < n; i++) {
    const token = tokens[rng.int(0, tokens.length - 1)];
    if (token === "CLOSE") {
      const name = open.pop();
      if (name) out += `</${name}>`;
      continue;
    }
    const name = /^<([a-zA-Z][^\s/>]*)[^<]*?(?<!\/)>$/.exec(token)?.[1];
    if (name && !VOID.has(name.toLowerCase())) open.push(name);
    out += token;
  }
  while (open.length > 0 && rng.random() < 0.9) out += `</${open.pop()}>`;
  return out;
}

const tokens = [...(dialect === "html" ? HTML_TOKENS : VUE_TOKENS), ...Array(15).fill("CLOSE")];
const corpus = Array.from({ length: count }, () => {
  const body = soup(tokens, rng.int(1, 30));
  return dialect === "html" ? body : `<template>${body}</template>\n`;
});

/** Mismatch shapes documented in DIVERGENCES.md, counted instead of failing. */
const QUIRKS: Record<string, RegExp> = dialect === "vue" ? {
  // Vue reads `<` as part of a tag name (`<a<C />`); we reject it in every dialect.
  "< in start tag": /<[a-zA-Z][^\s/>]*</,
  // Vue skips whitespace after `</` in a bogus comment's content.
  "</ whitespace": /<\/[ \t\n\f\r]/,
} : {
  // angular-html-parser does not close a void element before a doctype.
  "doctype after void": new RegExp(`${VOID_TAG}<!doctype`, "i"),
  // ... nor before a start tag, which then cannot close the void element's parent implicitly.
  "implicit close after void":
    new RegExp(
      `${VOID_TAG}<(?:address|article|aside|blockquote|div|dl|fieldset|footer|form|h[1-6]|header|hgroup|hr|main|nav|ol|p|pre|section|table|ul|tbody|tfoot|tr|td|th|li|dt|dd|rb|rt|rtc|rp|optgroup|option)\\b`,
      "i",
    ),
  // Its tokenizer cannot see the namespace, so SVG `<title>` is raw text there, against its own tag table.
  "svg title": /<svg\b(?:(?!<\/svg>)[^])*<title/i,
};

const oracle = dialect === "html" ? htmlOracle : vueOracle;
const theirs = corpus.map((source) => {
  try {
    return oracle(source);
  } catch (e) {
    return `error: oracle threw ${e}`;
  }
});
/**
 * What Vue's parser drops even with `whitespace: "preserve"`:
 * whitespace-only text as the first or last child (outside `pre` and raw text), and the `v-pre` prop.
 */
function vueNormalize(nodes: any[], keepEdges = false, svg = false, vPre = false): any[] {
  const blank = (n: any) => n[0] === "t" && /^[ \t\n\f\r]*$/.test(n[1]);
  const out = nodes.filter((n, i) => keepEdges || !blank(n) || (i > 0 && i < nodes.length - 1));
  for (const n of out) {
    if (n[0] !== "el") continue;
    // Only the outermost `v-pre` is a directive; nested ones are plain attributes Vue keeps.
    const opensVPre = !vPre && n[3].some((a: any) => a[0] === "v-pre");
    if (opensVPre) n[3] = n[3].filter((a: any) => a[0] !== "v-pre");
    // `pre` and raw text are HTML-only in Vue (`<textarea>` in SVG is an element like any other).
    const inSvg = n[1] === "foreignObject" ? false : svg || n[1] === "svg";
    const raw = !svg && ["pre", "textarea", "title", "script", "style"].includes(n[1]);
    n[4] = vueNormalize(n[4], keepEdges || raw, inSvg, vPre || opensVPre);
  }
  return out;
}
const mine = ours(dialect, corpus).map((sig) =>
  dialect === "vue" && typeof sig !== "string" ? vueNormalize(sig) : sig,
);

const tally: Record<string, number> = { same: 0, bothError: 0, lenient: 0, oursError: 0, mismatch: 0 };
const failures: string[] = [];
corpus.forEach((source, i) => {
  const [a, b] = [mine[i], theirs[i]];
  const aError = typeof a === "string";
  const bError = typeof b === "string";
  let kind: string;
  if (aError && bError) kind = "bothError";
  else if (bError) kind = "lenient";
  else if (aError) kind = "oursError";
  else kind = JSON.stringify(a) === JSON.stringify(b) ? "same" : "mismatch";
  if (kind === "oursError" || kind === "mismatch") {
    const quirk = process.env.NO_QUIRKS ? undefined : Object.keys(QUIRKS).find((name) => QUIRKS[name].test(source));
    if (quirk) kind = `quirk: ${quirk}`;
    else failures.push(`--- ${kind} #${i}\n${JSON.stringify(source)}\nours:   ${JSON.stringify(a)}\noracle: ${JSON.stringify(b)}`);
  }
  tally[kind] = (tally[kind] ?? 0) + 1;
});

console.log(failures.slice(0, 10).join("\n"));
if (process.env.ERRORS) {
  const messages: Record<string, number> = {};
  for (const sig of [...mine, ...theirs]) {
    if (typeof sig !== "string") continue;
    const key = sig.replace(/ at \d+.*| ".*/, "");
    messages[key] = (messages[key] ?? 0) + 1;
  }
  console.log(messages);
}
console.log(`${dialect} count=${count} seed=${seed}`, tally);
process.exit(failures.length > 0 ? 1 : 0);
