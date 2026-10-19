# AGENTS.md - AI Assistant Guide for oxc-htmlish-parser

A loose, span-faithful parser for HTML and HTML-based template languages (htmlish), producing a typed AST.
It is the parsing layer for oxc's htmlish formatter (`oxc_formatter_htmlish`).

## The one rule

Parse behavior targets each dialect's own parser, not the spec.

- html: `angular-html-parser` (what Prettier parses with), with Prettier's options
- vue: `@vue/compiler-core`
- Where Prettier errors on input the dialect accepts (`<DIV></div>`, unknown entities, `{{ a<b }}` in Vue), that is a bug to fix here.
- Truly broken structure (unmatched end tag, unterminated start tag) is an error, as in Prettier; the parse stops at the first one.

Every deliberate difference is an entry in `DIVERGENCES.md`.

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
- `src/ast.rs`: the AST.
  Not-yet-parsed dialect constructs slot in as new `Node` / `AttributeKind` variants.

## Testing

| Command | Covers |
| --- | --- |
| `cargo test` | error table (`src/parser.rs`), AST snapshots + span nesting (`tests/ast.rs`) |
| `cargo insta review` | accept snapshot changes after an intentional AST change |

- `tests/fixtures/html/*.html` parse as HTML, `tests/fixtures/vue/*.vue` as Vue; one compact tree per fixture.
- Add a fixture when a change touches structure or facts.
