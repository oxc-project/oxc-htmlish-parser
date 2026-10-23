# AGENTS.md - AI Assistant Guide for oxc-htmlish-parser

A loose, span-faithful parser for htmlish (HTML, Vue, Angular, Svelte, and Astro), producing a typed AST.
Svelte and Astro are not parsed yet.
It is the parsing layer for oxc's htmlish formatter (`oxc_formatter_htmlish`).

## The one rule

Parse behavior targets each dialect's own parser (the oracle), not the spec.

| Dialect | Oracle |
| --- | --- |
| html | `angular-html-parser` with Prettier's options; implicit end tags follow WHATWG |
| angular | `angular-html-parser` with Prettier's angular options; ICU expressions and `@let` are still text |
| vue | `@vue/compiler-core` |

Where Prettier errors on valid input the dialect accepts (`<DIV></div>` in HTML, unknown entities, `{{ a<b }}` in Vue), that is a bug to fix here.

### One reading for every dialect

Dialects share the HTML tokenizer's reading (names, comments, end tags, raw text ends).
A dialect reads a shape its own way only where valid code depends on it
(`{{ a<b }}` and `</Transition>` in Vue, `/>` on any element in Angular);
a shape it accepts only as a by-product of its tokenizer follows the shared reading
(an unterminated `{{` is an error in Angular too).
Where the shared reading would drop or alter what the user wrote, the shape is an error
(`<a / b>`, a quote in an attribute name, a template expression a printer would re-space).

Both are our decisions: following by-products multiplies branches for no working template,
and a parse error leaves the source as written.

### Errors

Broken structure (unmatched end tag, unterminated start tag) is an error, as in Prettier; the parse stops at the first one.
Implicit closing is not: an `.html` file is often a template or a fragment, and where an element closes changes the tree, not the source.
In HTML, WHATWG's tree wins where both readings parse;
an implicit close the oracle does not make is dropped where it would turn a written end tag into an error (`<p><figure></figure></p>`).

### Deciding an oracle difference

An oracle difference is never matched by reflex.
Before changing the parser either way, decide it in order:

1. Is there a norm? What builds the DOM from the source: browsers (WHATWG) for HTML, the dialect's compiler for the rest.
   `angular-html-parser` forks Angular's compiler, so its tag table is the norm in Angular.
2. Does the oracle deviate from it?
3. Is the deviation intent or accident?
   Intent is backed by a test, a declared table entry, an issue or explicit code: keep.
   Accident is a by-product of the machinery (a lookup that misses, a token left on a stack) with an unambiguous reading under the norm: drop.

Every drop is an entry in `DIVERGENCES.md`, with a fixture and (if the differential generates the shape) a `quirks` exemption in `run.ts`.
A keep against the HTML tokenizer goes in its last section, so that it is not reopened.

## Scope

- No tree construction beyond the tag table (`src/parser/tags.rs`):
  omitted start tags (`<tbody>`) are never inserted, and broken structure is an error, not repaired.
- No cooked values: spans into the original source; entities are never decoded.
- Facts that need no formatting option belong here (whitespace around text, how a tag was written, directive parts, `pre`'s ignored first line break);
  formatter policy (CSS display, whitespace sensitivity) does not.
- No dependency on `oxc_parser`: expressions are spans.
  Dialects that need a JS parse to find an expression's end take a validator from the caller.

## Testing

| Command | Covers |
| --- | --- |
| `cargo test` | error table (`src/parser/mod.rs`), AST snapshots + span nesting (`tests/ast.rs`) |
| `cargo insta review` | accept snapshot changes after an intentional AST change |
| `just differential <html\|vue\|angular> [count] [seed]` | structural fuzz against the oracles (`angular-html-parser`, `@vue/compiler-dom`) |
| `just differential-files <html\|vue\|angular> <file>...` | the same comparison on real files |
| `just compare <html\|vue\|angular> <rev> <file>...` | this tree against `<rev>` on real files: what now fails, parses, or reads differently |

First run: `npm i` in `tasks/differential`.

- `tests/fixtures/<dialect>/*` parse as that dialect (`html`, `vue`, `angular`); one compact tree per fixture.
- Add a fixture when a change touches structure or facts.
- The differential compares a signature (`tasks/sig/src/main.rs`, mirrored in `run.ts`);
  what it generates and fails on is in `run.ts`'s header.
  Each dialect is one entry there: oracle, normalizations, `quirks` (shapes in `DIVERGENCES.md`) and `pending` (not parsed yet).
  `NO_QUIRKS=1` shows exempted mismatches, `ALL=1` every failure, `ERRORS=1` the error messages by count.
- Before committing a change to how a shape is read, run `compare` (`--list <file>` for large corpora) on local repos:
  a file that parsed and now fails is a regression to explain, and a template language in `.html` (Nunjucks, Askama, ...)
  shows what a reading does to source a printer must not alter.
