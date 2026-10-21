# AGENTS.md - AI Assistant Guide for oxc-htmlish-parser

A loose, span-faithful parser for HTML and HTML-based template languages (htmlish), producing a typed AST.
It is the parsing layer for oxc's htmlish formatter (`oxc_formatter_htmlish`).

## The one rule

Parse behavior targets each dialect's own parser, not the spec.

- html: `angular-html-parser` (what Prettier parses with), with Prettier's options
- angular: `angular-html-parser` with Prettier's angular options (blocks, `{{ }}`, `/>` on every element).
  Only control flow blocks and `{{ }}` so far; ICU expressions and `@let` are still text
- vue: `@vue/compiler-core`
- Where Prettier errors on valid input the dialect accepts (`<DIV></div>`, unknown entities, `{{ a<b }}` in Vue), that is a bug to fix here.
- Dialects share one reading of the source; a dialect reads a shape its own way only where valid code in it depends on that reading
  (`{{ a<b }}` and `</Transition>` in Vue, `/>` on any element in Angular).
  A shape a dialect accepts only as a by-product of its tokenizer, with no valid use, follows the shared rule
  (an unterminated `{{` is an error in Angular as in Vue, though Angular renders it as text).
  This is our decision: following every such by-product would multiply per-dialect branches for no template that works.
- The shared rule is the HTML tokenizer's reading (names, comments, end tags, raw text ends),
  unless that reading would drop or alter what the user wrote: then the shape is an error
  (`<a / b>`, whose `/` a printer cannot keep; a quote in an attribute name, a template expression a printer would re-space).
  This is our decision too: one reading every dialect already knows, and a parse error leaves the source as written.
- Truly broken structure (unmatched end tag, unterminated start tag) is an error, as in Prettier; the parse stops at the first one.

An oracle difference is never matched by reflex.
Before changing the parser either way, decide it in order:

1. Is there a norm? WHATWG for HTML, the dialect's compiler for the rest.
2. Does the oracle deviate from it?
3. Is the deviation intent or accident?
   Intent is backed by a test, a declared table entry, an issue or explicit code: keep.
   Accident is a by-product of the machinery (a lookup that misses, a token left on a stack) with an unambiguous reading under the norm: drop.

Every drop is an entry in `DIVERGENCES.md`, with a fixture and (if the differential generates the shape) a `quirks` exemption in `run.ts`.

## Scope

- No WHATWG tree construction: only the tag table in `src/parser/tags.rs` (void, raw text, implicit closing).
  Omitted start tags (`<tbody>`) are never inserted.
- No cooked values: spans into the original source; entities are never decoded.
- Facts that need no formatting option belong here (whitespace around text, how a tag was written, directive parts, `pre`'s ignored first line break);
  formatter policy (CSS display, whitespace sensitivity) does not.
- No dependency on `oxc_parser`: expressions are spans.
  Dialects that need a JS parse to find an expression's end take a validator from the caller.

## Architecture

- `src/parser/`: one pass over the bytes builds the tree; there is no token stream.
  - `mod.rs`: the entry and the builder's core (scanning and tree helpers).
    `Dialect` maps to private `Rules` flags; a Vue file with a root `<html>` / `<!doctype html>` restarts with HTML rules.
  - `text.rs`, `markup.rs` (comments, CDATA, doctype), `element.rs` (tags, attributes, raw text): the builder by construct.
  - `angular.rs`: Angular's `{{ }}` end and control flow blocks.
  - `tags.rs`: the tag table. `vue.rs`: Vue directive name splitting.
- `src/ast.rs`: the AST, every node in one list linked by `NodeId` (parent, next sibling, children),
  so the formatter walks up and down and keys its tables by `NodeId`.
  Elements carry the tag table's verdict (`key`, `void`, `content`, `ignores_first_lf`) and the end tag's name as written.
  Not-yet-parsed dialect constructs slot in as new `NodeKind` / `AttributeKind` variants.

## Testing

| Command | Covers |
| --- | --- |
| `cargo test` | error table (`src/parser/mod.rs`), AST snapshots + span nesting (`tests/ast.rs`) |
| `cargo insta review` | accept snapshot changes after an intentional AST change |
| `just differential <html\|vue\|angular> [count] [seed]` | structural fuzz against the oracles (`angular-html-parser`, `@vue/compiler-dom`) |
| `just differential-files <html\|vue\|angular> <file>...` | the same comparison on real files |

First run: `npm i` in `tasks/differential`.

- `tests/fixtures/html/*.html` parse as HTML, `tests/fixtures/angular/*.html` as Angular, `tests/fixtures/vue/*.vue` as Vue; one compact tree per fixture.
- Add a fixture when a change touches structure or facts.
- The differential compares a signature (`tasks/sig/src/main.rs`, mirrored in `run.ts`):
  names, attributes, Vue directive parts, end-tag presence, raw text and comment contents.
  `run.ts` holds one entry per dialect (tokens, oracle, normalization, quirks); a new dialect is a new entry.
  - Inputs are token soups, half of them mutated byte-wise (`<`, `>`, `/`, quotes, ...) so that broken tag internals come up.
  - It fails on a tree mismatch, on our error where the oracle parses, on an AST invariant we break (`tests/check`, shared with `tests/ast.rs`),
    and on an oracle node kind the signature does not know; the oracle's error where we parse is counted as lenient.
  - Oracle-side normalizations (what Vue drops even with `whitespace: "preserve"`, split texts) live in `run.ts`;
    mismatches documented in `DIVERGENCES.md` are exempted by shape (`quirks`; `NO_QUIRKS=1` shows them),
    and constructs not parsed yet are counted as `pending`.
  - `ALL=1` prints every failure, `ERRORS=1` the error messages by count.
