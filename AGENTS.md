# AGENTS.md - AI Assistant Guide for oxc-htmlish-parser

A loose, span-faithful parser for HTML and HTML-based template languages (htmlish), producing a typed AST.
It is the parsing layer for oxc's htmlish formatter (`oxc_formatter_htmlish`).

## The one rule

Parse behavior targets each dialect's own parser, not the spec.

- html: `angular-html-parser` (what Prettier parses with), with Prettier's options
- vue: `@vue/compiler-core`
- Where Prettier errors on input the dialect accepts (`<DIV></div>`, unknown entities, `{{ a<b }}` in Vue), that is a bug to fix here.
- Truly broken structure (unmatched end tag, unterminated start tag) is an error, as in Prettier; the parse stops at the first one.

An oracle difference is never matched by reflex.
Before changing the parser either way, decide it in order:

1. Is there a norm? WHATWG for HTML, the dialect's compiler for the rest.
2. Does the oracle deviate from it?
3. Is the deviation intent or accident?
   Intent is backed by a test, a declared table entry, an issue or explicit code: keep.
   Accident is a by-product of the machinery (a lookup that misses, a token left on a stack) with an unambiguous reading under the norm: drop.

Every drop is an entry in `DIVERGENCES.md`, with a fixture and (if the differential generates the shape) a `QUIRKS` exemption.

## Scope

- No WHATWG tree construction: only the tag table in `src/tags.rs` (void, raw text, implicit closing).
  Omitted start tags (`<tbody>`) are never inserted.
- No cooked values: spans into the original source; entities are never decoded.
- Facts that need no formatting option belong here (whitespace around text, how a tag was written, directive parts, `pre`'s ignored first line break);
  formatter policy (CSS display, whitespace sensitivity) does not.
- No dependency on `oxc_parser`: expressions are spans.
  Dialects that need a JS parse to find an expression's end take a validator from the caller.

## Architecture

- `src/parser.rs`: one pass over the bytes builds the tree; there is no token stream.
  `Dialect` maps to private `Rules` flags; a Vue file with a root `<html>` / `<!doctype html>` restarts with HTML rules.
- `src/tags.rs`: the tag table.
- `src/vue.rs`: Vue directive name splitting.
- `src/ast.rs`: the AST, every node in one list linked by `NodeId` (parent, next sibling, children),
  so the formatter walks up and down and keys its tables by `NodeId`.
  Elements carry the tag table's verdict (`key`, `void`, `content`, `ignores_first_lf`) and the end tag's name as written.
  Not-yet-parsed dialect constructs slot in as new `NodeKind` / `AttributeKind` variants.

## Testing

| Command | Covers |
| --- | --- |
| `cargo test` | error table (`src/parser.rs`), AST snapshots + span nesting (`tests/ast.rs`) |
| `cargo insta review` | accept snapshot changes after an intentional AST change |
| `just differential <html\|vue> [count] [seed]` | structural fuzz against the oracles (`angular-html-parser`, `@vue/compiler-dom`) |

First run: `npm i` in `tasks/differential`.

- `tests/fixtures/html/*.html` parse as HTML, `tests/fixtures/vue/*.vue` as Vue; one compact tree per fixture.
- Add a fixture when a change touches structure or facts.
- The differential compares a signature (`tasks/sig/src/main.rs`, mirrored in `run.ts`):
  names, attributes, Vue directive parts, end-tag presence, raw text and comment contents.
  It fails on a tree mismatch or on our error where the oracle parses; the oracle's error where we parse is counted as lenient.
  Oracle-side normalizations (what Vue drops even with `whitespace: "preserve"`, split texts) live in `run.ts`;
  mismatches documented in `DIVERGENCES.md` are exempted by shape (`QUIRKS`; `NO_QUIRKS=1` shows them).
