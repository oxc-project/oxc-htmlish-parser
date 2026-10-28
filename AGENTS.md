# AGENTS.md - AI Assistant Guide for oxc-htmlish-parser

A loose, span-faithful parser for htmlish (HTML, Vue, Angular, Svelte, and Astro), producing a typed AST.
Svelte and Astro are not parsed yet.
It is the parsing layer for oxc's htmlish formatter (`oxc_formatter_htmlish`).

## The one rule

Parse behavior targets each dialect's oracle (not the spec), judged against its norm.

| Dialect | Oracle (the tree compared) | Norm (what reads the template; "without an error" means it reports none) |
| --- | --- | --- |
| html | `angular-html-parser` 10.12.0, with Prettier's options | WHATWG: parse5 7.3.0 reports its tokenizer's errors and a few of its tree builder's; the others are judged from the spec |
| vue | `@vue/compiler-dom` 3.5.44 `parse`, in SFC mode | its `compile` of the root `<template>`'s content |
| angular | `@angular/compiler` 22.0.6 `HtmlParser` | its `parseTemplate` |

- A parse the oracle accepts may still fail to compile (a `<script>` in a Vue template): the norm's step is what "without an error" means.
  A warning (`[Vue warn]`) is no error.
- parse5 7.3.0 predates WHATWG's customizable `<select>`, which keeps the markup in it: for `select`, the spec is the norm.
- A dialect with a compiler takes it as the oracle: what the compiler reads is what the template does.
  Prettier reads Vue and Angular with `angular-html-parser` too; where that differs from their compilers, the compiler wins.
  For Vue the differences are wholesale (`angular-html-parser` matches end tags case-sensitively, keeps HTML's implicit closing, and reports `{{ a<b }}`),
  for Angular the fork's own.
- HTML has no such compiler: browsers recover from any input, and an `.html` file is often a template whose source no browser reads.
  Prettier's parser is the oracle, and WHATWG the norm it is judged against.
- Versions follow Prettier's pin (3.9.10): the `angular-html-parser` it bundles, the `@angular/compiler` it depends on.
  A fix on an oracle's `main` is followed only as a `DIVERGENCES.md` entry citing it.
- Not parsed yet: Angular's ICU expressions and start-tag comments, an error for now, and `@let`, read as text.

## Deciding a difference

A difference is ours against the oracle, or the oracle against the norm where ours follows the oracle (a keep, once weighed).
It is never matched by reflex.
Check every claim with `just probe <dialect> <source>` (ours, the oracle, the norm's errors, what browsers build, Prettier's output at the pin),
and back the entry with what it prints; `just probe all <source>` shows every dialect's reading, for step 4.
Only a difference a printer could carry into its output is a reading to decide;
one in how the AST holds a part printed as written (a node kind, a dropped part kept as text) is recorded as `representation`.
Decide a reading in order; the first step that applies settles it:

1. Would a printer working from the oracle's reading change what the norm reads? Follow the norm.
   - What the norm reads is its tokens (tags, comments, text, attributes), always, and its tree, as "Trees" says.
     Whitespace a printer adds or removes is the formatter's to judge (its whitespace rules), not a reading.
2. Does the oracle deviate from the norm by accident? Drop it.
   - Intent is backed by a test, a declared table entry, an issue or explicit code: keep.
   - Accident is a by-product of the machinery (a lookup that misses, a token left on a stack, a branch the norm has and the oracle lacks)
     with an unambiguous reading under the norm.
3. Does the norm read the input without an error, where the oracle reports one, or ours would by the reading another dialect has? Parse it, as the norm reads it.
   - An error is safe (the input is left as written), but a gap.
     Leaving one needs an entry for its cost: a rare shape, and a branch to read it.
4. Otherwise (the norm reports an error too, or several readings pass), a reading already followed wins over a new one, in this order:
   the norm's rule for a nearby valid shape (Angular's compiler closes a custom element at `/>`, so `<div/>` closes too),
   then "Trees" for trees, then the HTML tokenizer's for tokens.
   - It never overrides 1: a dialect reads a shape its own way where its compiler does (`{{ a<b }}` in Vue, `</ div>` in Vue and Angular).
   - Where that reading would let a printer alter what the user wrote (re-space a template's expression, drop a `/` or a `</br>`),
     the shape stays an error.
   - A reading made at a start tag holds for every continuation:
     Vue's compiler nests `<option>a<option>`, since `<option>a<option>b</option></option>` is valid there, so ours nests it too.

In Vue and Angular the oracle is the norm: step 2 never applies, and a difference from the oracle is decided by 1, 3, 4 or `representation`.
When unsure which step applies, keep the oracle's reading: a drop needs grounds, a keep does not.

Every drop is an entry in `DIVERGENCES.md` naming the deciding step (`decided by:`), its fixtures (`fixture:`)
and (if the differential generates the shape) its `quirks` exemption in `tasks/differential/dialects.ts` (`runner:`).
`decided by:` is the step that settled the reading, `Trees`, or `representation`; a step 3 parse that keeps as text what the norm drops is `3, representation`.
A keep that was weighed goes in its last section, with the step that kept it, so that it is not reopened.
`cargo test` holds the file to this (`tests/divergences.rs`): the lines, the fixtures they name, the runner names against the quirks,
and every `DIVERGENCES.md#slug` the sources cite.
A gap left for its cost (step 3) is an entry in its dialect's section too (`decided by: 3, cost`, its fixture the input that errors);
one not parsed yet is in "The one rule" and the differential's `pending`.

### Bumping the oracles

Prettier's pin moves the oracles: `angular-html-parser` and `@angular/compiler` follow its `package.json`, the rest the versions that ship with it.

1. Bump `tasks/differential/package.json` (exact versions, excluded from Renovate) and the table above.
2. Run `just differential` for every dialect, over several seeds, and `just differential-files` on local repos.
3. A quirk that no longer fires is an entry to re-decide: delete it when the oracle now reads as we do.
   A new mismatch is decided as above, never exempted by reflex.

## Trees

The tree is the written nesting, closed by the tag table (`src/parser/tags.rs`): void elements, raw text, implicit closing.
Step 1 holds a printer to a norm's tree only where the input has no parse error.

- Implicit closing follows the norm: WHATWG's optional end tags in HTML, Angular's table in Angular; Vue's compiler closes nothing implicitly.
  WHATWG's closers that would turn a written end tag into an error (`p` at `figure`: `<p><figure></figure></p>`) are left out, as in the oracle.
- What a norm inserts without a parse error (`html`, `head`, `body`, `tbody`) is not inserted:
  a printer adds no tags, so the norm inserts them into the output again.
- What a norm rebuilds on a parse error (anything its tree builder does on one: foster parenting, the adoption agency, nested `a`,
  breaking out of foreign content) is not rebuilt, as in the oracle:
  a template's source is no page a browser reads, and the rebuilt tree is none of the template's.
  The written nesting is formatted as is, best effort.
- Broken structure is an error, as in the oracles; the parse stops at the first one:
  an unterminated start tag, or an unmatched end tag, which no open element takes or which would close one that cannot close implicitly
  (the `</b>` of `<b><i>x</b>`, where the adoption agency rebuilds).
  An implicit close never is: where an element closes changes the tree, not the source.

## Scope

- No cooked values: spans into the original source; entities are never decoded.
- Facts that need no formatting option belong here (whitespace around text, how a tag was written, directive parts, `pre`'s ignored first line break);
  formatter policy (CSS display, whitespace sensitivity) does not.
- No dependency on `oxc_parser`: expressions are spans.
  Dialects that need a JS parse to find an expression's end take a validator from the caller.

## Testing

| Command | Covers |
| --- | --- |
| `cargo test` | error table (`src/parser/mod.rs`), AST snapshots + span nesting (`tests/ast.rs`), `DIVERGENCES.md` against what it names (`tests/divergences.rs`) |
| `just probe <html\|vue\|angular\|all> <source>` | one input through every reader: ours (normalized, and raw where it differs), the oracle, the norm, browsers (html), Prettier at the pin |
| `cargo insta review` | accept snapshot changes after an intentional AST change |
| `just differential <html\|vue\|angular> [count] [seed]` | structural fuzz against the oracles (`angular-html-parser`, `@vue/compiler-dom`, `@angular/compiler`) |
| `just differential-files <html\|vue\|angular> <file>...` | the same comparison on real files |
| `just compare <html\|vue\|angular> <rev> <file>...` | this tree against `<rev>` on real files: what now fails, parses, or reads differently |

First run: `npm i` in `tasks/differential`.
CI (`.github/workflows`) runs check, test, clippy, doc and typos, and `differential.yml` the fuzz for every dialect over seeds 0 to 2.

- `tests/fixtures/<dialect>/*` parse as that dialect (`html`, `vue`, `angular`); one compact tree per fixture.
- Add a fixture when a change touches structure or facts.
- The differential compares a signature (`tasks/sig/src/main.rs`, mirrored in `tasks/differential/dialects.ts`);
  what it generates and fails on is in `run.ts`'s header.
  Each dialect is one entry in `dialects.ts`: oracle, normalizations, `quirks` (shapes in `DIVERGENCES.md`) and `pending` (not parsed yet).
  `NO_QUIRKS=1` shows exempted mismatches, `ALL=1` every failure, `ERRORS=1` the error messages by count.
- Before committing a change to how a shape is read, run `compare` (`--list <file>` for large corpora) on local repos:
  a file that parsed and now fails is a regression to explain, and a template language in `.html` (Nunjucks, Askama, ...)
  shows what a reading does to source a printer must not alter.
