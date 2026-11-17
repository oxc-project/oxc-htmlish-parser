// The dialects the differential compares: each one's oracle, our signature in its shape,
// and the shapes DIVERGENCES.md documents, shared by `run.ts` and `probe.ts`.
// Angular's oracle is its compiler's `HtmlParser`, and its `parseTemplate` for the readings of `@for` and `@if`
// and of a microsyntax, whose errors fail the template; a block it reports reads as `UNKNOWN`, which matches any of ours.
import { spawnSync } from "node:child_process";
import path from "node:path";
import { HtmlParser, Lexer, ParseLocation, ParseSourceFile, ParseSourceSpan, Parser as ExpressionParser, parseTemplate } from "@angular/compiler";
import { parse as parseVue } from "@vue/compiler-dom";
import { parseHtml } from "angular-html-parser";

const ROOT = path.resolve(import.meta.dirname, "../..");
const BIN = path.join(ROOT, "target/debug/sig-batch");

export type Sig = unknown[] | string;

/** The text of an oracle's `span` in `source`. */
const sliceOf = (source: string, span: any): string => source.slice(span.start.offset, span.end.offset);

/** Angular's template compiler, with the options it compiles a component's template with: the norm of `angular`. */
export const compileTemplate = (source: string) => parseTemplate(source, "template.html", { preserveWhitespaces: true });

/** A node kind the signature does not know: the harness, not the input, needs fixing. */
export class UnknownNode extends Error {}

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
  "<div v-pre>", "<div v-pre:a>", "<div v-pre.m>", "<div a=b c>", "<div v-model.trim='m'>",
  "<C v-slot.m=\"p\">", "<C #.m=\"p\">", "<C v-slot:.m>", "<C #[a.b].c=\"p\">",
  "<li v-for=\"(x, i) in xs\">", "<li v-for='x of xs'>", "<li v-for=\" { a, b }, k , i  in\n o \">",
  "<li v-for=\"[a, b], k, i, j in xs\">", "<li v-for=\"(, i) in xs\">", "<li v-for=\"x inxs\">", "<li v-for=\" in xs\">",
  "<pre>", "<pre>\n", "<textarea>\n{{ t }}</textarea>", "<title>{{ t }} <b></title>",
  "<script>a < b</script>", "<style>a>b{}</style>", "<svg>", "<svg><style>a<b</style></svg>",
  "<Title><b>t</b></Title>", "<tItle><b>t</b></tItle>", "<tiTLE><b></tiTLE>",
  "<Textarea>{{ t }}<b></Textarea>", "<Style><b>a</b></Style>",
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
  "@let a = b;", "@let x = '}';",
  "{n, plural, =0 {none} other {{{ n }} items}}", "{a, select, x {<b>X} y {<i>Y</i>}}", "{a, select, x {{b, plural, =1 {one}}}}",
  "{n, plural, =0 {", "} other {", "}}",
  "<div ngNonBindable>", "<p title=\"a {{ b }} {{ '}}' }}\">", "<p title='{{ a // \"c }}'>", "<a href={{x}}>",
  "<p [a]=\"{{ b\" c=\"{{ <b> }}\">",
  "<li *ngFor=\"let x of xs | p: 1 : n; index as i trackBy f\">", "<p *ngIf=\"a ? b : c | async as d; else e\">",
  "<p *ngIf=\"f('; ,', [a, b]) && !c?.d in {e: `${ g }`}\">", "<p *x=\"a /b/ c\">",
  "<div a // c\n>", "<div /* c */ a>", "<div a=b/* c */>",
  // Expressions: root pipes, pipes not at the root, comments, a regular expression
  "{{ a | p: x : y | q }}", "{{ (a | p) }}", "{{ a ? b : c | p }}", "{{ a | p: b ? c : d }}", "{{ `${a | p}` }}",
  "{{ /a|b/.test(c) | p }}", "{{ a || b | p }}", "{{ a | p // c }}", "{{ a | }}", "{{ a | p q }}",
  "<p [a]=\"b | p: 1\">", "<p [a]=\"{b: c} | p\">", "<p [a]=\"a : b\">", "<p [a]=\"f(a | p)\">",
  "{{ (a ? b : c) | p }}", "{{ ((a ? b : c)) | p: (x ? y : z) : w ? v : u }}", "{{ (a ? b : c).d | p }}", "{{ (a) ? (b) : c }}", "{{ (x => a ? b : c) | p }}", "{{ f | p: x => a ? b : c }}",
  "<p (a)=\"b(); c = d;\">", "<p (a)=\"b;; c ? d() : e // f\">", "<p (a)=\"b | c\">", "<p (a)=\"\">", "<p (a)=\"f(); // c\">", "{{ f(a,) }}", "<p (a)=\"f(a, b,)\">", "{{ [a,] }}", "{{ g({a: 1,}) }}", "<input [(a)]=\"b.c | d\">", "<input [(a)]=\"b // c\">",
];

// ---
// Oracle signatures, the shape `tasks/sig/src/main.rs` documents.

function stripQuotes(value: string): string {
  return /^(["']).*\1$/s.test(value) ? value.slice(1, -1) : value;
}

const LEADING_LF = /^(\r\n|\n|\r)/;
const IGNORES_FIRST_LF = new Set(["pre", "textarea", "listing"]);
export const VOID = new Set("area,base,br,col,embed,hr,img,input,link,meta,param,source,track,wbr".split(","));
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
  const slice = (span: any) => sliceOf(source, span);
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
/** `@angular/compiler`'s token types for an interpolation in a text and in an attribute value. */
const INTERPOLATION = 8;
const ATTR_VALUE_INTERPOLATION = 18;

/** `@angular/compiler`'s tree, parsed with the options its template compiler passes. */
function angularCompilerSig(source: string): Sig {
  const { rootNodes, errors } = angularParser.parse(source, "template.html", {
    tokenizeExpansionForms: true,
    tokenizeBlocks: true,
    tokenizeLet: true,
  });
  if (errors.length > 0) return `error: ${errors[0].msg}`;
  // Its template compiler reports a microsyntax error as the template's
  let bindingError: string | null = null;
  // Compiled only for a template with a block to compare
  let readings: Map<number, unknown> | undefined;
  const slice = (span: any) => sliceOf(source, span);
  const nodes = (nodes: any[]): unknown[][] => nodes.flatMap((n) => (n.constructor.name === "Text" ? node(n) : [node(n)])) as unknown[][];
  function node(n: any): any {
    switch (n.constructor.name) {
      case "Element": {
        // As written: Angular's name carries the namespace (`:svg:rect`)
        const name = /^<([^\t-\x20\xa0>\/<'"=]+)/.exec(slice(n.startSourceSpan))![1];
        // A void or self-closed element ends at its start tag
        const end = n.endSourceSpan;
        const ended = end !== null && end.start.offset !== n.startSourceSpan.start.offset;
        // A value's span leaves its quotes out
        // Start-tag comments go among the attributes, in source order; ours leaves out a `//` one's trailing whitespace
        const attrs = [
          ...n.attrs.map((a: any) => {
            const attr: unknown[] = [slice(a.keySpan), a.valueSpan ? slice(a.valueSpan) : null];
            // Its compiler binds the `{{ }}` in a name it reads as no binding of another kind (`parseAttribute`)
            const interpolations = (a.valueTokens ?? []).filter((t: any) => t.type === ATTR_VALUE_INTERPOLATION && t.parts.length === 3).map((t: any) => t.parts[1]);
            if (interpolations.length > 0 && !BOUND_NAME.test(attr[0] as string)) {
              attr.push(interpolations, interpolations.map(expressionReading));
            }
            if (PROPERTY_NAME.test(attr[0] as string) && a.valueSpan) {
              attr.push(["ex", expressionReading(attr[1] as string)]);
            }
            if (EVENT_NAME.test(attr[0] as string) && a.valueSpan) {
              attr.push(["ac", actionReading(attr[1] as string)]);
            }
            if (attr[0].startsWith("*") && a.valueSpan) {
              const bindings = templateBindings(attr[0].slice(1), attr[1]);
              if (typeof bindings === "string") bindingError ??= bindings;
              attr.push(["tb", bindings]);
            }
            return [a.sourceSpan.start.offset, attr];
          }),
          ...n.comments.map((c: any) => [c.sourceSpan.start.offset, ["comment", slice(c.sourceSpan).replace(/[ \t\n\f\r]+$/, "")]]),
        ]
          .sort((a, b) => a[0] - b[0])
          .map((a) => a[1]);
        return elementSig(source, n, name, ended, attrs, nodes(n.children));
      }
      // Its interpolations are tokens; one without its `}}` is text to us
      case "Text":
        return n.tokens
          .map((t: any) => {
            if (t.type !== INTERPOLATION || t.parts.length !== 3) return ["t", slice(t.sourceSpan)];
            return ["i", slice(t.sourceSpan), expressionReading(t.parts[1])];
          })
          .filter((t: any) => t[1] !== "");
      // Trimmed, as Angular keeps it
      case "Comment":
        return ["c", n.value ?? ""];
      // Our parameters are trimmed (`Block::parameters`).
      case "Block": {
        // Its template compiler drops a `@for` or `@if` whose parameters it reports: no reading to compare
        const read = ["for", "if", "else if", "switch", "case"].includes(n.name);
        const reading = read ? ((readings ??= blockReadings(source)).get(n.startSourceSpan.start.offset) ?? UNKNOWN) : NO_READING;
        return ["b", n.name, n.parameters.map((p: any) => p.expression.replace(/^[ \t\n\f\r]+|[ \t\n\f\r]+$/g, "")), nodes(n.children), reading];
      }
      // Our value is trimmed (`LetDeclaration::value`).
      case "LetDeclaration": {
        const value = n.value.replace(/[ \t\n\f\r]+$/, "");
        return ["l", n.name, value, expressionReading(value.replace(/^[ \t\n\f\r]+/, ""))];
      }
      // Ours are trimmed (`Icu`, `IcuCase`); Angular's lexer skips the whitespace at the start of a case.
      case "Expansion":
        return ["icu", n.switchValue.trim(), n.type.trim(), n.cases.map((c: any) => ["case", c.value, nodes(c.expression)])];
      default:
        throw new UnknownNode(`unexpected node ${n.constructor.name}`);
    }
  }
  const sig = mapNodes(nodes(rootNodes), mergeTexts);
  return bindingError === null ? sig : `error: ${bindingError}`;
}

/** The names Angular's `parseAttribute` reads as a binding other than an interpolation. */
const BOUND_NAME = /^(?:\*|i18n(?:-|$)|bind-|let-|ref-|#|on-|bindon-|@|\[\(.+\)\]$|\[.+\]$|\(.+\)$)/;
/** What an oracle could not tell: it matches any reading of ours (`withUnknownAsOurs`). */
export const UNKNOWN = "?";
const NO_READING = [null, null, null, [], null, [null, null]];

/** `oracle` with each `UNKNOWN` in it replaced by what `ours` holds there. */
export function withUnknownAsOurs(ours: unknown, oracle: unknown): unknown {
  if (oracle === UNKNOWN) return ours;
  if (!Array.isArray(oracle) || !Array.isArray(ours)) return oracle;
  return oracle.map((value, i) => withUnknownAsOurs(ours[i], value));
}

/** The `@for` and `@if` readings of Angular's template compiler (`parseTemplate`), by the block's start: `tasks/sig`'s `reading`. */
// A block whose start holds an error it reports reads as `UNKNOWN`
function blockReadings(source: string): Map<number, unknown> {
  const readings = new Map<number, unknown>();
  let parsed: any;
  try {
    parsed = compileTemplate(source);
  } catch {
    return readings;
  }
  const errors: number[] = (parsed.errors ?? []).map((e: any) => e.span.start.offset);
  const set = (n: any, reading: unknown[]) => {
    const { start, end } = n.startSourceSpan;
    const reported = errors.some((at) => start.offset <= at && at < end.offset);
    readings.set(start.offset, reported ? UNKNOWN : reading);
  };
  const slice = (span: any) => sliceOf(source, span);
  const walk = (nodes: any[]) => {
    for (const n of nodes) {
      // An expression's reading, from the source the compiler parsed (`null` for none)
      const read = (e: any) => (e === null || e === undefined ? null : typeof e.source === "string" ? expressionReading(e.source.trim()) : UNKNOWN);
      if (n.constructor.name === "ForLoopBlock") {
        const lets = n.contextVariables.filter((v: any) => v.keySpan.start.offset !== v.keySpan.end.offset).map((v: any) => [slice(v.keySpan), slice(v.valueSpan)]);
        set(n, [n.item.name, n.expression.source.trim(), n.trackBy.source.trim(), lets, null, [read(n.expression), read(n.trackBy)]]);
      } else if (n.constructor.name === "IfBlockBranch") {
        set(n, [null, null, null, [], n.expressionAlias?.name ?? null, [read(n.expression), null]]);
      } else if (n.constructor.name === "SwitchBlock" || n.constructor.name === "SwitchBlockCase") {
        set(n, [null, null, null, [], null, [read(n.expression), null]]);
      }
      for (const value of Object.values(n)) {
        if (Array.isArray(value)) walk(value.filter((x) => x?.sourceSpan !== undefined));
        else if (value?.constructor?.name?.endsWith("Block")) walk([value]);
      }
    }
  };
  walk(parsed.nodes);
  return readings;
}

const expressionParser = new ExpressionParser(new Lexer());
/** The directive's own key is placed before the value, so that its spans tell it apart. */
const KEY_OFFSET = -1e6;

/** A property binding's name (`[x]`) or a two-way one's (`[(x)]`), both binding an expression,
 * as `parseAttribute` reads it: the delimiters by the prefix alone, so `[(a)-]` binds nothing. */
const PROPERTY_NAME = /^(?:\[\(.+\)\]|\[(?!\().+\])$/;

/** An event binding's name (`(x)`), as `parseAttribute` reads it. */
const EVENT_NAME = /^\(.+\)$/;

/** An action (an event binding's value) as Angular's compiler reads it, in `tasks/sig`'s shape (`["ac", …]`):
 * `[[statement, …], comment]`, each statement `[text, conditional]`; `null` for an empty one, `UNKNOWN` where it reports an error. */
function actionReading(code: string): unknown {
  const location = new ParseLocation(new ParseSourceFile(code, "t.html"), 0, 0, 0);
  const { ast, errors } = expressionParser.parseAction(code, new ParseSourceSpan(location, location), 0);
  if (errors.length > 0) return UNKNOWN;
  const comment = (expressionParser as any)._commentStart(code);
  const commentText = comment === null ? null : code.slice(comment).replace(/[\x00-\x20\xa0]+$/, "");
  if (ast.constructor.name === "EmptyExpr") return commentText === null ? null : [[], commentText];
  const statements = ast.constructor.name === "Chain" ? ast.expressions : [ast];
  return [statements.map((n: any) => pieceOf(code, n)), commentText];
}

/** A piece of code: its text, and whether its root, parens around it aside, is a conditional. */
function pieceOf(code: string, n: any): unknown[] {
  let inner = n;
  while (inner.constructor.name === "ParenthesizedExpression") inner = inner.expression;
  return [code.slice(n.span.start, n.span.end), inner.constructor.name === "Conditional"];
}

/** An expression (`{{ }}`'s, a property binding's) as Angular's compiler reads it, in `tasks/sig`'s shape (`expression`);
 * `null` for an empty one, `UNKNOWN` where it reports an error: ours finds some of them only (`None`, the formatter keeps it as written). */
function expressionReading(code: string): unknown {
  const location = new ParseLocation(new ParseSourceFile(code, "t.html"), 0, 0, 0);
  const { ast, errors } = expressionParser.parseBinding(code, new ParseSourceSpan(location, location), 0);
  if (errors.length > 0) return UNKNOWN;
  if (ast.constructor.name === "EmptyExpr") return null;
  const piece = (n: any) => pieceOf(code, n);
  const pipes: unknown[] = [];
  let root = ast;
  while (root.constructor.name === "BindingPipe") {
    pipes.unshift([root.name, root.args.map(piece)]);
    root = root.exp;
  }
  const comment = (expressionParser as any)._commentStart(code);
  return [piece(root), pipes, comment === null ? null : code.slice(comment).replace(/[\x00-\x20\xa0]+$/, "")];
}

/** A `*x` value's microsyntax as Angular's compiler reads it, in `tasks/sig`'s shape; the error it reports. */
function templateBindings(key: string, value: string): unknown[] | string {
  const location = new ParseLocation(new ParseSourceFile(value, "t.html"), 0, 0, 0);
  const { templateBindings, errors } = expressionParser.parseTemplateBindings(key, value, new ParseSourceSpan(location, location), KEY_OFFSET, 0);
  if (errors.length > 0) return errors[0].msg;
  const text = (k: any) => (k === null ? null : k.span.start < 0 ? null : value.slice(k.span.start, k.span.end));
  return templateBindings.map((b: any) => {
    if (b.constructor.name === "ExpressionBinding") {
      const source = b.value === null ? null : b.value.source;
      return ["e", text(b.key), source, source === null ? null : expressionReading(source)];
    }
    const start = b.sourceSpan.start;
    return start >= 0 && /^let(?![\w$])/.test(value.slice(start)) ? ["l", text(b.key), text(b.value)] : ["a", text(b.value), text(b.key)];
  });
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
          const f = p.forParseResult;
          const vFor = f ? [f.value?.content ?? null, f.key?.content ?? null, f.index?.content ?? null, f.source.content] : null;
          return [p.rawName, value(p, p.rawName.length), p.name, arg, p.arg ? !p.arg.isStatic : false, modifiers, vFor];
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
/** `v-pre`, and `v-pre:a` / `v-pre.m`: the directive is named `pre`. */
const isVPre = (a: any) => /^v-pre(?:$|[:.])/.test(a[0]);

function vueNormalize(nodes: any[], keepEdges = false, svg = false, vPre = false, root = true): any[] {
  const blank = (n: any) => n[0] === "t" && /^[ \t\n\f\r]*$/.test(n[1]);
  const out = nodes.filter((n, i) => keepEdges || !blank(n) || (i > 0 && i < nodes.length - 1));
  for (const n of out) {
    if (n[0] !== "el") continue;
    // Only the outermost `v-pre` is a directive; nested ones are plain attributes Vue keeps.
    const opensVPre = !vPre && n[3].some(isVPre);
    if (opensVPre) n[3] = n[3].filter((a: any) => !isVPre(a));
    // `pre` and raw text are HTML-only in Vue (`<textarea>` in SVG is an element like any other).
    const inSvg = n[1] === "foreignObject" ? false : svg || n[1] === "svg";
    // A root block is raw text, unless it is a `<template>` in HTML.
    const htmlTemplate = n[1] === "template" && !n[3].some((a: any) => a[0] === "lang" && !["", "html", null].includes(a[1]));
    const raw = (root && !htmlTemplate) || (!svg && ["pre", "textarea", "title", "script", "style"].includes(n[1]));
    n[4] = vueNormalize(n[4], keepEdges || raw, inSvg, vPre || opensVPre, false);
  }
  return out;
}

/**
 * Our bogus comments as Vue drops them (but `</ x>`, which it keeps):
 * a text after one joins the text before it, so that text's source (what Vue's signature holds) spans the comment.
 */
function vueDropsBogus(nodes: any[]): any[] {
  const dropped = (n: any) => n?.[0] === "bc" && n[2] !== "</";
  const out: any[] = [];
  nodes.forEach((n, i) => {
    if (!dropped(n)) return out.push(n);
    const next = nodes.slice(i + 1).find((m) => !dropped(m));
    if (out.at(-1)?.[0] === "t" && next?.[0] === "t") out.at(-1)[1] += `<${n[2] === "<!" ? "!" : ""}${n[1]}>`;
  });
  return out;
}

/** Rewrites every node list of a signature, children first. */
function mapNodes(nodes: any[], f: (nodes: any[]) => any[]): any[] {
  for (const n of nodes) {
    if (n[0] === "el") n[4] = mapNodes(n[4], f);
    if (n[0] === "b") n[3] = mapNodes(n[3], f);
    if (n[0] === "icu") for (const c of n[3]) c[2] = mapNodes(c[2], f);
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
/** A start tag, up to any point outside its quoted values: a shape after it is in the tag, not in text or a quoted value. */
const IN_START_TAG = `<[a-zA-Z](?:[^<>"']|"[^"]*"|'[^']*')*?`;

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
export type Shape = RegExp | ((source: string) => boolean);

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
 * and it reads a bogus comment `<?` opens, and CDATA, as text, and trims comments.
 */
function angularShape(nodes: any[]): any[] {
  const out = nodes
    .filter((n) => n[0] !== "d" && !(n[0] === "bc" && n[2] === "<!"))
    .map((n) => (n[0] === "x" ? ["t", n[1].slice(9, -3)] : n[0] === "bc" && n[2] === "<?" ? ["t", `<${n[1]}>`] : n));
  return bogusAsComment(mergeTexts(out)).map((n) => {
    if (n[0] === "c") return ["c", n[1].replace(/\r\n?/g, "\n").trim()];
    // Its lexer skips the whitespace at the start of a case
    if (n[0] === "icu") {
      for (const c of n[3]) {
        const first = c[2][0];
        if (first?.[0] !== "t") continue;
        first[1] = first[1].replace(/^[\t-\r \xa0]+/, "");
        if (first[1] === "") c[2].shift();
      }
    }
    // Its lexer reads `{{ }}` in RCDATA as text; its compiler binds it all the same, as we read it
    if (n[0] === "el" && ["textarea", "title"].includes(n[1].toLowerCase())) return [...n.slice(0, 4), mergeTexts(n[4], true)];
    return n;
  });
}

export const DIALECTS: Record<string, DialectSpec> = {
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
      // It reads `<html:style>` as `style`, and `<svg:rect>` as SVG.
      "namespace prefix": /<[a-zA-Z][^\s/>:]*:[^\s/>]/,
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
      "unquoted value with a quote or =": new RegExp(`${IN_START_TAG}=[ \\t\\n\\f\\r]*[^\\s"'>][^\\s>]*["'=]`),
      "empty comment": /<!---?>/,
      "raw text end after whitespace": /<\/[ \t\n\f\r]+(?:script|style|textarea|title)\b/i,
      // A quote, `<` or `=` in a tag name, or a quote or `<` in an attribute name (after `= ` it is a value, not a name)
      "punctuation in a name": new RegExp(`${PUNCTUATION_IN_TAG_NAME.source}|${IN_START_TAG}(?<![=\\s])\\s+[^\\s/>="'<]*["'<]`),
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
      // ... and an ICU expression right after it becomes its child.
      "ICU after void": new RegExp(`${VOID_TAG}\\{(?!\\{)`, "i"),
      // Its lexer starts `@let` in an ICU case where a token starts.
      "@let in an ICU case": /\{[^{}]*,[^{}]*,[^]*\{[^]*@let\s/,
      // Angular reads the whitespace after a body-less `@case` into its span.
      "fallthrough case": /@(?:case|default)\b(?:[^{}@]*?\))?[ \t\n\f\r]+@/,
    },
    mutationQuirks: {
      // Angular ends a tag name at `= < ' "` too; ours, printed as written, keeps them in it.
      "punctuation in a name": PUNCTUATION_IN_TAG_NAME,
    },
  },
  vue: {
    tokens: VUE_TOKENS,
    wrap: (body) => `<template>${body}</template>\n`,
    oracle: vueOracle,
    // Vue drops the bogus comments a `<!` or `<?` opens; we keep them, as all content.
    normalize: (sig) => vueNormalize(mapNodes(sig, (nodes) => mergeTexts(bogusAsComment(vueDropsBogus(nodes))))),
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

export function ours(dialect: string, inputs: string[]): Sig[] {
  const build = spawnSync("cargo", ["build", "-q", "-p", "sig"], { cwd: ROOT, encoding: "utf8" });
  if (build.status !== 0) throw new Error(build.stderr);
  const r = spawnSync(BIN, [dialect], { input: JSON.stringify(inputs), encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`sig-batch failed (a panic?):\n${r.stderr}`);
  return JSON.parse(r.stdout);
}

/** The first shape of `table` that `source` has. */
export const matchShape = (table: Record<string, Shape>, source: string) =>
  Object.keys(table).find((name) => {
    const shape = table[name];
    return typeof shape === "function" ? shape(source) : shape.test(source);
  });
