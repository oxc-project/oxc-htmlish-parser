# Divergences from the oracles

The parse-behavior oracles:

- html and angular: `angular-html-parser` 10.13, with the options Prettier passes for each
- vue: `@vue/compiler-core` (Prettier reads Vue through `angular-html-parser` too; where they differ, Vue wins)

Each entry is a place where we deliberately do not match, with the input that shows it.
Where the oracle errors and we parse, Prettier fails on the file and nothing can depend on it.
`runner:` names the shape `tasks/differential/run.ts` exempts (some only in mutated inputs and files, where whole-tag tokens do not limit them);
entries without one are never generated there, fixtures pin them.
Entries are grouped by the dialects they apply to.
Where the oracles agree against the HTML tokenizer and we keep their reading, the last section says why.

## All dialects

### `<` and `=` are name characters

In tag names, attribute names and Vue directive arguments, as in the HTML tokenizer (quotes too, in tag names):
`<p=e>` is an element named `p=e`, `<a<C />` one named `a<C`, and `<div <span>` a `div` with an attribute named `<span`.
`angular-html-parser` reports these (or splits off an attribute without a name), and Vue reports a `<` in a plain attribute name;
no valid template uses them, and reading them as browsers do keeps every dialect on one rule.
`runner: punctuation in a name`

### Quotes in attribute names are an error

The HTML tokenizer reads `<a b"c>` as an attribute named `b"c`.
Such a name is almost always a template's expression nesting the quotes of a value (`content="{{ f("-") }}"`, in Nunjucks or Askama files):
read as browsers do, the printer would re-space what follows the cut value, changing the expression.
We report it, as `angular-html-parser` and Vue (in a plain attribute name) do.
Angular's bracketed names may hold quotes (DIVERGENCES.md#bracketed-attribute-names-end-at-).
`runner: punctuation in a name`

### `</` without a name right after it is a bogus comment

As in the HTML tokenizer and Vue (`</ div>`, `</3>`); htm's `<//>` aside, in HTML.
`angular-html-parser` reads `</ div >` as an end tag.
Vue's comment content skips the whitespace after `</`; ours is the raw data.
`runner: </ whitespace`

### A comment closes at `--!>` too

As in the HTML tokenizer (and Astro): `<!-- a --!> b -->` is the comment ` a `, then the text ` b -->`.
`angular-html-parser` and Vue (and Svelte) read on to the next `-->`.
`--!>` is a parse error no template uses, so every dialect follows the tokenizer, as with `<!-->`.
`runner: --!>`

### A script holds script end tags inside `<!-- <script>`

In the HTML tokenizer (and Astro), a `<script` after `<!--` in a script hides the next `</script>` (until `-->`):

```html
<script><!-- document.write("<script src=a.js></script>"); --></script>
<!-- angular-html-parser, Vue: the script closes inside the string, then an unexpected </script> error; ours (browsers): one script -->
```

Valid HTML that Prettier errors on, so we read it as browsers do in every dialect.
Vue SFC blocks are the exception: Vue's SFC parser reads them, not a browser, and they end at their first end tag.
`runner: script escape`

## HTML and Angular

### Unquoted attribute values end at whitespace or `>` only

`angular-html-parser` also ends them at `/ ' " = <`.
Browsers and Vue do not.

```html
<a href=/foo/bar>x</a>
<!-- angular-html-parser: error -->
<img src=a.png/>
<!-- angular-html-parser: src="a.png", self-closed; ours (browsers): src="a.png/" -->
```
`runner: unquoted value with a quote or =`

### End tags match case-insensitively

`<DIV></div>` is an error in `angular-html-parser` (it compares names as written).

### Entities are not validated

`&unknown;` and malformed numeric references are errors in `angular-html-parser`.
Text and values are never decoded here, so they are plain text.

### Whitespace is HTML ASCII whitespace

`angular-html-parser` also treats NBSP and the controls U+000E–U+001F as whitespace inside tags.
We use tab, LF, FF, CR and space, as the HTML tokenizer does.

### `foreignObject` children are HTML

`angular-html-parser` declares `preventNamespaceInheritance` on `foreignObject`,
but the case-insensitive lookup (`foreignobject`) misses the entry, so its children stay SVG.

### SVG `<title>` holds markup

`angular-html-parser`'s tag table declares SVG `title` as parsable data (only the HTML `title` is escapable raw text),
but its tokenizer only sees an explicit `svg:` prefix, so `<svg><title>` is raw text there.
We follow the declared intent, as browsers do.
`runner: svg title`

```html
<svg><title><b>t</b></title></svg>
<!-- angular-html-parser: title holds the text "<b>t</b>"; ours: title holds a b element -->
```

### A void element closes before anything that follows it

`angular-html-parser` keeps a void element on its stack until the next token.
A doctype right after it becomes its child,
and a start tag right after it cannot implicitly close the void element's parent.
`runner: doctype after void`, `runner: implicit close after void`

```html
<th><img src=x><td>
<!-- angular-html-parser: td inside th; ours: th, td siblings -->
```

### A `prefix:` is part of the element name

`angular-html-parser` reads `<html:style>` as `style` in the HTML namespace and `<svg:rect>` as an SVG `rect`.
The HTML tokenizer has no namespace prefixes: both are unknown HTML elements, with markup inside, as in browsers.

```html
<p><html:style><b>x</b></html:style></p>
<!-- angular-html-parser: a style element holding the text "<b>x</b>"; ours: html:style holding a b element -->
```

### An attribute name may start with `=`

`<a =b>` is an attribute named `=b`, as in browsers;
`angular-html-parser` reads an empty name with the value `b`.
`runner: = starts an attribute name`

### Raw text closes only where the name follows the slash

`angular-html-parser` allows whitespace between `</` and the name of the end tag that closes raw text (`</ script>`), as it does in any end tag.
The HTML tokenizer and Vue close raw text only at `</` followed right away by the name, so `</ script>` inside a script is content.

```html
<script>const s = "</ script>";</script>
<!-- angular-html-parser: the script closes at </ script>, then an unexpected </script> error; ours (browsers, Vue): one script -->
```

The end tag then runs to its first `>`, as any end tag does (`</script foo>` closes the script).
`runner: raw text end after whitespace`

### A comment may close right after its opening

`<!-->` and `<!--->` are empty comments in the HTML tokenizer and Vue; `angular-html-parser` reads on to the next `-->`.

```html
<!-->a-->
<!-- angular-html-parser: one comment holding ">a"; ours (browsers, Vue): an empty comment, then the text "a-->" -->
```
`runner: empty comment`

### An end tag runs to its first `>`

As in the HTML tokenizer (and Vue), what follows the name up to the `>` is ignored: `</div foo>` closes the `div`.
`angular-html-parser` reports it.

## HTML

### `/>` on non-void HTML elements is ignored

Prettier passes `canSelfClose: true`, so every element self-closes.
Browsers ignore the `/` except on void and foreign (SVG / MathML) elements, and so do we.
The written `/>` is kept as `Element::self_closing`.

```html
<div />after</div>
<!-- Prettier: <div/> then text "after", then an unexpected </div> error -->
<!-- ours: div containing "after" -->
```
`runner: /> on non-void`

### An element whose `/>` was ignored closes on an ancestor's end tag

`angular-html-parser` lets an ancestor's end tag close only the `closedByParent` set (`p`, `li`, `td`, ...).
Since the previous entry leaves `<my-el />` open, that set alone would make the common shape below an error;
browsers close it there.

```html
<div><my-el /></div>
<!-- ours: my-el closes at </div>, without an end tag -->
```

### Bracketed attribute names end at `>`

`angular-html-parser` reads an attribute name opened with `[` while its brackets are open, `>` included (`[class.[&>svg]:w-4]`),
which Angular templates rely on for Tailwind's arbitrary variants; we read it so in Angular only.
In HTML the `>` ends the tag, as in browsers.
`runner: bracketed attribute name`

### Implicit end tags follow WHATWG's optional tags

Browsers build an HTML page from its source, so an end tag goes where WHATWG lets it be omitted.
`angular-html-parser`'s table misses some; it is Angular's compiler's, which builds Angular's DOM, so Angular keeps it.

- `tr` also closes at `tbody` and `tfoot` (we insert no `tbody`), `option` and `optgroup` at `hr`;
  `caption` and `colgroup` close at what may follow them in a table, `body` at its parent's end tag.
- A start tag closes through the elements above it that may go without an end tag:
  `<li><p>a<li>`, `<tr><td>a<tr>`; `angular-html-parser` looks at the direct parent only.
- An ancestor's end tag closes `head` and `body`: `<html><body>x</html>` is an error in `angular-html-parser`.

```html
<table><tr><td>a<tr><td>b</table>
<!-- angular-html-parser: the second tr inside the first td; ours (browsers): two rows -->
```

WHATWG's start tags that close an element the oracle keeps open are not followed where a written end tag would then be an error
(AGENTS.md: implicit closing adds no error):

- `p` does not close at `details`, `dialog`, `figcaption`, `figure`, `menu` or `search`: `<p><figure></figure></p>` parses.
- `head` closes only at `<body>`, not at everything it cannot hold:
  real pages put elements there before an explicit `</head>` (a browser moves them to the body).

So `<p>a<figure>`, and `<head><title>t</title><p>` with both `</head>` and `<body>` omitted,
read the second element inside the first, though both are valid HTML.
`runner: WHATWG optional end tags`

## Angular

### A block without a body ends at its parameters

`angular-html-parser` reads the whitespace after the parameters into a block's start (it may precede the `{`),
so a fallthrough `@case` without a body takes the whitespace up to the next `@case` into its span.
Every other span ends at the construct's last delimiter (`>`, `{`, `;`, `)`), and the whitespace after it is a text.

```html
@switch (a) { @case (1) @case (2) {x} }
<!-- angular-html-parser: "@case (1) ", then "@case (2) {x}"; ours: "@case (1)", the text " ", then "@case (2) {x}" -->
```
`runner: fallthrough case`

### An unterminated `{{` is an error

Angular reads a `{{` without its `}}` before the next tag or the end of the input as text (kept for backward compatibility).
No template relies on it (Angular escapes a literal `{{` as `{{ '{{' }}`), so it is an error, as in Vue;
a tag ends the search even inside a string, as in Angular.
`runner: unterminated interpolation`

## Vue

### `{{ }}` closes at the first `}}`, tags included

`angular-html-parser` tracks quotes inside interpolation and ends it at a tag start,
so `{{ a<b }}` is an error in Prettier.
Vue's tokenizer only looks for `}}`; so do we (`{{ '}}' }}` closes inside the string, as in Vue).

### No implicit closing, case-insensitive end tags

Vue has no implicit end tags (`<p><div></div></p>` nests), and matches `</transition>` to `<Transition>`.
Prettier's Vue options keep the HTML implicit-close table and match case-sensitively.

### `v-pre`

Inside `v-pre`, `{{ }}` is text and directive names are plain attributes,
including the `v-pre` element's own directives, written before `v-pre` or after.
Prettier formats both.

### A root `<html>` or `<!doctype html>` makes the file HTML

As Prettier reads a Vue file: it is an HTML page, not an SFC, and parses with HTML rules (`{{ }}` and directives stay Vue).
Vue reads such a root element as a custom block holding raw text.
`runner: html page`

### A directive name without its argument is a plain attribute

`v-slot:`, `v-bind:` and a lone `:` / `@` / `#` are attributes, not directives with an empty argument as in Vue:
a broken name keeps its source as written, with no part to format.
`runner: directive without argument`

### Bogus comments are kept

What `<!` or `<?` opens without a comment or CDATA is a bogus comment up to its `>`, as in the HTML tokenizer (`<!x>`, `<?x?>`, `<!- c -->`);
we keep it as a comment, as all content.
Vue drops it, reads some as text (`<!- c -->`, `<! />`), and reads `<!>` on to a later `>`.
`run.ts` drops the ones Vue drops from our signature.
`runner: bogus comment`

### A `/` that no `>` follows in a start tag is an error

`<a / b>` and `<C a="b"/ c>` are reported, as in `angular-html-parser`; Vue and browsers ignore the `/`.
`runner: / in start tag`

### `</br>` is an error

As in `angular-html-parser`; Vue reads it as `<br>`, as HTML tree construction does, which we do not do.
`runner: </br>`

### `{{ }}` in RCDATA stays inside it

In `<title>` and `<textarea>`, `<{{ t }}` holds the text `<` and an interpolation, as anywhere else in them,
and an interpolation without its `}}` before the end tag is an error.
Vue's tokenizer misses an interpolation right after a `<` there, and reads an unterminated one on past the end tag.
`runner: {{ }} in RCDATA`

## Kept: the oracles over the HTML tokenizer

Not divergences: recorded so that they are not reopened.
Svelte's and Astro's readings are noted for the dialects to come.

### `xmp`, `iframe`, `noembed`, `noframes`, `noscript` and `plaintext` hold markup

Browsers read their content as raw text (`noscript` only with scripting on).
None of them holds markup in valid HTML (obsolete, or nothing allowed) but `noscript`, whose markup is what a printer formats.
Svelte reads markup too; Astro, raw text.

### CDATA is read in HTML content too

Browsers read `<![CDATA[a>b]]>` outside SVG and MathML as a bogus comment up to the first `>`, then text.
We keep the section whole, as `angular-html-parser` (and Angular's compiler) does:
the cut would leave the rest as text a printer re-spaces.
Vue and Svelte report it.

### Duplicate attributes are kept

Browsers drop all but the first, which would drop source.
`angular-html-parser` and Astro keep them; Vue and Svelte report them.
