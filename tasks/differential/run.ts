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
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { HtmlParser } from "@angular/compiler";
import { parse as parseVue } from "@vue/compiler-dom";
import { parseHtml } from "angular-html-parser";

const ROOT = path.resolve(import.meta.dirname, "../..");
const BIN = path.join(ROOT, "target/debug/sig-batch");

type Sig = unknown[] | string;

/** A node kind the signature does not know: the harness, not the input, needs fixing. */
class UnknownNode extends Error {}

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
  "</ div>", "<!-- c --!>", "</br>", "<C :=\"o\" v-bind:=\"p\">", "<title><{{ t }}</title>",
];

const ANGULAR_TOKENS = [
  ...TEXT,
  "<div>", "<span>", "<p>", "<ul>", "<li>", "<my-cmp>", "<my-cmp />", "<div />", "<br>", "<pre>", "<pre>\n",
  "<div [a]=\"b\" (c)=\"d()\" *ngIf=\"e\" #f>", "<!-- c -->",
  "{{ a }}", "{{ '}}' }}", "{{ a // b }}", "a@b.com",
  "@if (a) {", "@else if (b) {", "@else {", "@for (i of xs; track i) {", "@empty {",
  "@switch (a) {", "@case (1)", "@case (2) {", "@default {", "@defer (on idle) {", "@placeholder {",
  "</ div>", "<!-- c --!>", "<!-->", "<style>a</ style >", "<svg:g>", "<svg:rect/>", "<svg><title><b>t</b></title></svg>",
  "<div\u00a0a=b\u00a0c>", "{{ a",
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

/**
 * An element's signature from an oracle's node: `name` as written, `ended` when an end tag closed it.
 * The oracles drop a text that was only the ignored line break; the start tag's end tells whether there was one.
 */
function elementSig(source: string, n: any, name: string, ended: boolean, attrs: unknown[], children: unknown[][]): unknown[] {
  // Not in an implicit namespace (`:svg:pre`)
  const lfTag = !/^:\w+:/.test(n.name) && !n.isSelfClosing && IGNORES_FIRST_LF.has(name.toLowerCase());
  const lf = lfTag && (leadingLf(children) || LEADING_LF.test(source.slice(n.startSourceSpan.end.offset)));
  let flags = n.isSelfClosing ? "/" : "";
  if (ended && !n.isSelfClosing) flags += "e";
  if (lf) flags += "lf";
  return ["el", name, flags, attrs, children];
}

/** `angular-html-parser`, with the options Prettier passes for `html`. */
function angularHtmlParserSig(source: string): Sig {
  const { rootNodes, errors } = parseHtml(source, { canSelfClose: true, allowHtmComponentClosingTags: true });
  if (errors.length > 0) return `error: ${errors[0].msg}`;
  const slice = (span: any) => source.slice(span.start.offset, span.end.offset);
  const nodes = (nodes: any[]): unknown[][] => nodes.map(node);
  function node(n: any): unknown[] {
    switch (n.kind) {
      case "element": {
        const attrs = n.attrs.map((a: any) => [slice(a.keySpan), a.valueSpan ? stripQuotes(slice(a.valueSpan)) : null]);
        return elementSig(source, n, slice(n.nameSpan), n.endSourceSpan !== null, attrs, nodes(n.children));
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
        throw new UnknownNode(`unexpected node ${n.kind}`);
    }
  }
  return mapNodes(nodes(rootNodes), mergeTexts);
}

const angularParser = new HtmlParser();

/** `@angular/compiler`'s tree, parsed with the options its template compiler passes. */
function angularCompilerSig(source: string): Sig {
  const { rootNodes, errors } = angularParser.parse(source, "template.html", {
    tokenizeExpansionForms: true,
    tokenizeBlocks: true,
    tokenizeLet: true,
  });
  if (errors.length > 0) return `error: ${errors[0].msg}`;
  const slice = (span: any) => source.slice(span.start.offset, span.end.offset);
  const nodes = (nodes: any[]): unknown[][] => nodes.map(node);
  function node(n: any): unknown[] {
    switch (n.constructor.name) {
      case "Element": {
        // As written: Angular's name carries the namespace (`:svg:rect`)
        const name = /^<([^\t-\x20\xa0>\/<'"=]+)/.exec(slice(n.startSourceSpan))![1];
        // A void or self-closed element ends at its start tag
        const end = n.endSourceSpan;
        const ended = end !== null && end.start.offset !== n.startSourceSpan.start.offset;
        // A value's span leaves its quotes out
        const attrs = n.attrs.map((a: any) => [slice(a.keySpan), a.valueSpan ? slice(a.valueSpan) : null]);
        return elementSig(source, n, name, ended, attrs, nodes(n.children));
      }
      case "Text":
        return ["t", slice(n.sourceSpan)];
      // Trimmed, as Angular keeps it
      case "Comment":
        return ["c", n.value ?? ""];
      // Our parameters are trimmed (`Block::parameters`).
      case "Block":
        return ["b", n.name, n.parameters.map((p: any) => p.expression.replace(/^[ \t\n\f\r]+|[ \t\n\f\r]+$/g, "")), nodes(n.children)];
      // Not parsed yet (AGENTS.md): text on our side.
      case "Expansion":
      case "LetDeclaration":
        return ["t", slice(n.sourceSpan)];
      default:
        throw new UnknownNode(`unexpected node ${n.constructor.name}`);
    }
  }
  return mapNodes(nodes(rootNodes), mergeTexts);
}

/**
 * angular-html-parser splits escapable raw text at entities, and keeps `{{ }}` inside its text;
 * a run of texts (and, with `interpolations`, interpolations) is one text.
 */
function mergeTexts(nodes: unknown[][], interpolations = false): unknown[][] {
  const out: unknown[][] = [];
  for (const n of nodes) {
    const last = out.at(-1);
    const text = n[0] === "t" || (interpolations && n[0] === "i");
    if (text && last?.[0] === "t") last[1] = (last[1] as string) + (n[1] as string);
    else out.push(text ? ["t", n[1]] : n);
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
        throw new UnknownNode(`unexpected node type ${n.type}`);
    }
  }
  return nodes(ast.children);
}

/**
 * What Vue's parser drops even with `whitespace: "preserve"`:
 * whitespace-only text as the first or last child (outside `pre` and raw text), and the `v-pre` prop.
 */
function vueNormalize(nodes: any[], keepEdges = false, svg = false, vPre = false, root = true): any[] {
  const blank = (n: any) => n[0] === "t" && /^[ \t\n\f\r]*$/.test(n[1]);
  const out = nodes.filter((n, i) => keepEdges || !blank(n) || (i > 0 && i < nodes.length - 1));
  for (const n of out) {
    if (n[0] !== "el") continue;
    // Only the outermost `v-pre` is a directive; nested ones are plain attributes Vue keeps.
    const opensVPre = !vPre && n[3].some((a: any) => a[0] === "v-pre");
    if (opensVPre) n[3] = n[3].filter((a: any) => a[0] !== "v-pre");
    // `pre` and raw text are HTML-only in Vue (`<textarea>` in SVG is an element like any other).
    const inSvg = n[1] === "foreignObject" ? false : svg || n[1] === "svg";
    // A root block is raw text, unless it is a `<template>` in HTML.
    const htmlTemplate = n[1] === "template" && !n[3].some((a: any) => a[0] === "lang" && !["", "html", null].includes(a[1]));
    const raw = (root && !htmlTemplate) || (!svg && ["pre", "textarea", "title", "script", "style"].includes(n[1]));
    n[4] = vueNormalize(n[4], keepEdges || raw, inSvg, vPre || opensVPre, false);
  }
  return out;
}

/** Rewrites every node list of a signature, children first. */
function mapNodes(nodes: any[], f: (nodes: any[]) => any[]): any[] {
  for (const n of nodes) {
    if (n[0] === "el") n[4] = mapNodes(n[4], f);
    if (n[0] === "b") n[3] = mapNodes(n[3], f);
  }
  return f(nodes);
}

/** The oracles have no bogus comment kind. */
const bogusAsComment = (nodes: any[]) => nodes.map((n) => (n[0] === "bc" ? ["c", n[1]] : n));

// ---
// Dialects

/** `parser/tags.rs`'s implicit closing: the start tags that close an element opened right inside it. */
const P_CLOSERS = "address article aside blockquote div dl fieldset footer form h1 h2 h3 h4 h5 h6 header hgroup hr main nav ol p pre section table ul";
const RUBY = "rb rt rtc rp";
const ANGULAR_CLOSED_BY: Record<string, string> = {
  p: P_CLOSERS, thead: "tbody tfoot", tbody: "tbody tfoot", tfoot: "tbody", tr: "tr", td: "td th", th: "td th",
  li: "li", dt: "dt dd", dd: "dt dd", rb: RUBY, rt: RUBY, rp: RUBY, rtc: "rb rtc rp", optgroup: "optgroup", option: "option optgroup",
};
const ANGULAR_CLOSERS = [...new Set(Object.values(ANGULAR_CLOSED_BY).join(" ").split(" "))].join("|");

/** A void element stays open until the next token in angular-html-parser and Angular's compiler: a start tag right after it cannot close the void element's parent implicitly. */
const IMPLICIT_CLOSE_AFTER_VOID = new RegExp(`${VOID_TAG}<(?:${ANGULAR_CLOSERS})\\b`, "i");

/** A `<script` after `<!--` in a script, which hides its next `</script>` in browsers. */
const SCRIPT_ESCAPE = /<script\b[^>]*>(?:(?!<\/script)[^])*?<!--(?:(?!-->)[^])*?<script[\s/>]/i;
/** A quote, `<` or `=` in a tag name. */
const PUNCTUATION_IN_TAG_NAME = /<[a-zA-Z][^\s/>]*["'<=]/;

interface DialectSpec {
  tokens: readonly string[];
  /** The soup as a file of the dialect. */
  wrap: (body: string) => string;
  oracle: (source: string) => Sig;
  /** Our signature, as the oracle shapes it. */
  normalize: (sig: unknown[]) => unknown[];
  /** Mismatch shapes documented in DIVERGENCES.md (`runner:`), counted instead of failing. */
  quirks: Record<string, Shape>;
  /** Documented shapes whole-tag tokens never form, exempted in mutated inputs and files only (too broad for all). */
  mutationQuirks: Record<string, Shape>;
  /** Constructs not parsed yet (AGENTS.md), counted apart from divergences. */
  pending?: Record<string, Shape>;
}

/** A pattern, or a predicate where a regex cannot follow the reading. */
type Shape = RegExp | ((source: string) => boolean);

const WHATWG_CLOSED_BY: Record<string, string> = {
  ...ANGULAR_CLOSED_BY,
  tr: "tr tbody tfoot", optgroup: "optgroup hr", option: "option optgroup hr",
  caption: "colgroup thead tbody tfoot tr", colgroup: "colgroup thead tbody tfoot tr", head: "body",
};
const CLOSED_BY_PARENT = new Set("p tbody tfoot tr td th li dd rb rt rp rtc optgroup option head body caption colgroup".split(" "));

/** Whether a start tag closes other elements under WHATWG's optional end tags (HTML) than under `angular-html-parser`'s table. */
function whatwgClosing(source: string): boolean {
  const closes = (whatwg: boolean, el: string, name: string) =>
    ((whatwg ? WHATWG_CLOSED_BY : ANGULAR_CLOSED_BY)[el] ?? "").split(" ").includes(name);
  const stack: string[] = [];
  for (const [, end, raw, selfClosing] of source.matchAll(/<(\/?)([a-zA-Z][^\s/>]*)(?:[^>"']|"[^"]*"|'[^']*')*?(\/?)>/g)) {
    const name = raw.toLowerCase();
    if (end) {
      const i = stack.lastIndexOf(name);
      if (i !== -1) stack.length = i;
      continue;
    }
    let whatwg = stack.length;
    for (let i = stack.length - 1; i >= 0; i--) {
      if (closes(true, stack[i], name)) whatwg = i;
      if (!CLOSED_BY_PARENT.has(stack[i])) break;
    }
    const angular = stack.length > 0 && closes(false, stack.at(-1)!, name) ? stack.length - 1 : stack.length;
    if (whatwg !== angular) return true;
    stack.length = whatwg;
    if (!VOID.has(name) && !selfClosing) stack.push(name);
  }
  return false;
}


/**
 * Ours in the shape of Angular's AST (DIVERGENCES.md "How Angular's AST holds a node"): it drops doctypes,
 * and the bogus comments `<!` opens, which it reads as doctypes;
 * it reads a bogus comment `<?` opens, and CDATA, as text, and trims comments.
 */
function angularShape(nodes: any[]): any[] {
  const out = nodes
    .filter((n) => n[0] !== "d" && !(n[0] === "bc" && n[2] === "<!"))
    .map((n) => (n[0] === "x" ? ["t", n[1].slice(9, -3)] : n[0] === "bc" && n[2] === "<?" ? ["t", `<${n[1]}>`] : n));
  return bogusAsComment(mergeTexts(out, true)).map((n) => (n[0] === "c" ? ["c", n[1].replace(/\r\n?/g, "\n").trim()] : n));
}

const DIALECTS: Record<string, DialectSpec> = {
  html: {
    tokens: HTML_TOKENS,
    wrap: (body) => body,
    oracle: angularHtmlParserSig,
    normalize: (sig) => mapNodes(sig, bogusAsComment),
    quirks: {
      // `</` and whitespace open a bogus comment; angular-html-parser reads `</ div >` as an end tag.
      "</ whitespace": /<\/[ \t\n\f\r]/,
      // angular-html-parser does not close a void element before a doctype either.
      "doctype after void": new RegExp(`${VOID_TAG}<!doctype`, "i"),
      "implicit close after void": IMPLICIT_CLOSE_AFTER_VOID,
      // Its tokenizer cannot see the namespace, so SVG `<title>` is raw text there, against its own tag table.
      "svg title": /<svg\b(?:(?!<\/svg>)[^])*<title/i,
      // Implicit end tags follow WHATWG's optional tags, through omittable ancestors.
      "WHATWG optional end tags": whatwgClosing,
    },
    // What angular-html-parser reads otherwise, as mutations form it.
    mutationQuirks: {
      // It closes a comment at `-->` only ...
      "--!>": /--!>/,
      // ... and ends a script at its first `</script>`, even after `<!-- <script`.
      "script escape": SCRIPT_ESCAPE,
      "/> on non-void": new RegExp(`<(?!(?:${[...VOID].join("|")})[\\s/>])[a-zA-Z][^\\s/>]*(?:\\s[^<>]*)?/>`, "i"),
      "= starts an attribute name": /<[a-zA-Z][^<>]*[\s"']=/,
      "unquoted value with a quote or =": /=[ \t\n\f\r]*[^\s"'>][^\s>]*["'=]/,
      "empty comment": /<!---?>/,
      "raw text end after whitespace": /<\/[ \t\n\f\r]+(?:script|style|textarea|title)\b/i,
      "punctuation in a name": new RegExp(`${PUNCTUATION_IN_TAG_NAME.source}|\\s[^\\s/>="'<]*["'<]`),
      // It reads Angular's bracketed attribute names in HTML too.
      "bracketed attribute name": /\s\[[^\n=]*>/,
    },
  },
  angular: {
    tokens: ANGULAR_TOKENS,
    wrap: (body) => body,
    oracle: angularCompilerSig,
    normalize: (sig) => mapNodes(sig, angularShape),
    quirks: {
      "implicit close after void": IMPLICIT_CLOSE_AFTER_VOID,
      // Angular reads the whitespace after a body-less `@case` into its span.
      "fallthrough case": /@(?:case|default)\b(?:[^{}@]*?\))?[ \t\n\f\r]+@/,
    },
    mutationQuirks: {
      // Angular ends a tag name at `= < ' "` too; ours, printed as written, keeps them in it.
      "punctuation in a name": PUNCTUATION_IN_TAG_NAME,
    },
    pending: {
      "@let": /@let\s/,
      ICU: /\{[^{}]*,\s*(?:plural|select|selectordinal)\s*,/,
      "start-tag comment": /<[a-zA-Z][^<>]*(?:\/\/|\/\*)/,
    },
  },
  vue: {
    tokens: VUE_TOKENS,
    wrap: (body) => `<template>${body}</template>\n`,
    oracle: vueOracle,
    // Vue drops the bogus comments a `<!` or `<?` opens; we keep them, as all content.
    normalize: (sig) =>
      vueNormalize(mapNodes(sig, (nodes) => mergeTexts(bogusAsComment(nodes.filter((n) => n[0] !== "bc" || n[2] === "</"))))),
    quirks: {
      // Vue skips whitespace after `</` in a bogus comment's content.
      "</ whitespace": /<\/[ \t\n\f\r]/,
      // Inside `v-pre`, Vue names `v-bind:` without its colon, and drops its value.
      "directive without argument in v-pre": /\bv-pre\b[^]*\sv-[^\s=>/:]+:(?=[\s=>/])/,
      // Vue drops a void element's end tag; we keep it in the text.
      "</br>": new RegExp(`<\\/[ \\t\\n\\f\\r]*(?:${[...VOID].join("|")})[\\s/>]`),
    },
    mutationQuirks: {
      // Vue's parser ends a script at its first `</script>`; its compiler reports any script in a template.
      "script escape": SCRIPT_ESCAPE,
      // A root `<html>` or `<!doctype html>` makes the file HTML, as Prettier reads it.
      "html page": /<html\b|<!doctype\s+html\b/i,
      // Vue drops a bogus comment `<!` opens, or reads it as text or on to a later `>`.
      "bogus comment": /<!(?!--|\[CDATA\[)/i,
    },
  },
};

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

// ---

function ours(dialect: string, inputs: string[]): Sig[] {
  const build = spawnSync("cargo", ["build", "-q", "-p", "sig"], { cwd: ROOT, encoding: "utf8" });
  if (build.status !== 0) throw new Error(build.stderr);
  const r = spawnSync(BIN, [dialect], { input: JSON.stringify(inputs), encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`sig-batch failed (a panic?):\n${r.stderr}`);
  return JSON.parse(r.stdout);
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
/** The first shape of `table` that `source` has. */
const matchShape = (table: Record<string, Shape>, source: string) =>
  Object.keys(table).find((name) => {
    const shape = table[name];
    return typeof shape === "function" ? shape(source) : shape.test(source);
  });

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
