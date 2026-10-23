# Divergences from the oracles

The parse-behavior oracles:

- html and angular: `angular-html-parser` 10.13, with the options Prettier passes for each
- vue: `@vue/compiler-core` (Prettier reads Vue through `angular-html-parser` too; where they differ, Vue wins)

Each entry is a place where we deliberately do not match: its heading names the concern, `Ours:` says what we do,
and the rest what the oracles do and why, with the input that shows it.
Where the oracle errors and we parse, Prettier fails on the file and nothing can depend on it.
`runner:` names the shape `tasks/differential/run.ts` exempts (some only in mutated inputs and files, where whole-tag tokens do not limit them);
entries without one are never generated there, fixtures pin them.
Entries are grouped by the dialects they apply to.
Where the oracles agree against the HTML tokenizer and we keep their reading, the last section says why.

## All dialects

### Name characters

Ours: `<` and `=` are part of tag names, attribute names and Vue directive arguments, as in the HTML tokenizer (quotes too, in tag names):
`<p=e>` is an element named `p=e`, `<a<C />` one named `a<C`, and `<div <span>` a `div` with an attribute named `<span`.
`angular-html-parser` reports these (or splits off an attribute without a name), and Vue reports a `<` in a plain attribute name;
no valid template uses them, and reading them as browsers do keeps every dialect on one rule.
`runner: punctuation in a name`

### Quotes in attribute names

Ours: an error, as in `angular-html-parser` and Vue (in a plain attribute name).
The HTML tokenizer reads `<a b"c>` as an attribute named `b"c`.
Such a name is almost always a template's expression nesting the quotes of a value (`content="{{ f("-") }}"`, in Nunjucks or Askama files):
read as browsers do, the printer would re-space what follows the cut value, changing the expression.
Angular's bracketed names may hold quotes (DIVERGENCES.md#bracketed-attribute-names).
`runner: punctuation in a name`

### End tag without a name

Ours: `</` without a name right after it opens a bogus comment, as in the HTML tokenizer and Vue (`</ div>`, `</3>`); htm's `<//>` aside, in HTML.
`angular-html-parser` reads `</ div >` as an end tag, as Angular's compiler does.
No template relies on it (an end tag is written `</div>`), so Angular keeps the shared reading too.
Vue's comment content skips the whitespace after `</`; ours is the raw data.
`runner: </ whitespace`

### Incorrectly closed comment

Ours: a comment closes at `--!>` too, as in the HTML tokenizer (and Astro): `<!-- a --!> b -->` is the comment ` a `, then the text ` b -->`.
`angular-html-parser` and Vue (and Svelte) read on to the next `-->`.
`--!>` is a parse error no template uses, so every dialect follows the tokenizer, as with `<!-->`.
`runner: --!>`

### Script data escapes

Ours: a `<script` after `<!--` in a script hides the next `</script>` (until `-->`), as in the HTML tokenizer (and Astro):

```html
<script><!-- document.write("<script src=a.js></script>"); --></script>
<!-- angular-html-parser, Vue: the script closes inside the string, then an unexpected </script> error; ours (browsers): one script -->
```

Valid HTML that Prettier errors on, so we read it as browsers do in every dialect.
Vue SFC blocks are the exception: Vue's SFC parser reads them, not a browser, and they end at their first end tag.
`runner: script escape`

## HTML and Angular

### Unquoted attribute values

Ours: they end at whitespace or `>`, as in browsers and Vue; in Angular, a `/` right before `>` self-closes the element.
`angular-html-parser` also ends them at `/ ' " = <`.

```html
<a href=/foo/bar>x</a>
<!-- angular-html-parser: error -->
<img src=a.png/>
<!-- angular-html-parser: src="a.png", self-closed; ours (browsers): src="a.png/" -->
```
`runner: unquoted value with a quote or =`

Angular's compiler ends every unquoted value at `/`, so the only valid template with one is `<cmp a=b/>`, which depends on it
(the Angular team keeps that reading, angular/angular#68804).
Every other `/` is an error in Angular and keeps the shared reading.

### End tag case

Ours: end tags match case-insensitively.
`<DIV></div>` is an error in `angular-html-parser` (it compares names as written).

### Character references

Ours: not validated; text and values are never decoded here, so they are plain text.
`&unknown;` and malformed numeric references are errors in `angular-html-parser`.

### Whitespace in tags

Ours: tab, LF, FF, CR and space, as the HTML tokenizer has it.
`angular-html-parser` also treats NBSP and the controls U+000E–U+001F as whitespace inside tags.

### `foreignObject` children

Ours: HTML, as in browsers and Angular's compiler.
`angular-html-parser` declares `preventNamespaceInheritance` on `foreignObject`,
but the case-insensitive lookup (`foreignobject`) misses the entry, so its children stay SVG.
Angular's compiler looks the exact name up first and reads them as HTML; the fork's case-insensitive lookup is the accident.

### SVG `title`

Ours: it holds markup, the declared intent, as browsers read it.
`angular-html-parser`'s tag table declares SVG `title` as parsable data (only the HTML `title` is escapable raw text),
but its tokenizer never applies it (Angular's own compiler does, for an explicit `svg:` prefix only, angular/angular#40259),
so `<svg><title>` is raw text there.
`runner: svg title`

```html
<svg><title><b>t</b></title></svg>
<!-- angular-html-parser: title holds the text "<b>t</b>"; ours: title holds a b element -->
```

### Void element end

Ours: a void element closes before anything that follows it.
`angular-html-parser` keeps a void element on its stack until the next token.
A doctype right after it becomes its child,
and a start tag right after it cannot implicitly close the void element's parent.
`runner: doctype after void`, `runner: implicit close after void`

```html
<th><img src=x><td>
<!-- angular-html-parser: td inside th; ours: th, td siblings -->
```

### Namespace prefixes

Ours: a `prefix:` is part of the element name: `<html:style>` and `<svg:rect>` are unknown HTML elements, with markup inside, as in browsers.
`angular-html-parser` reads `<html:style>` as `style` in the HTML namespace and `<svg:rect>` as an SVG `rect`.
The HTML tokenizer has no namespace prefixes.

```html
<p><html:style><b>x</b></html:style></p>
<!-- angular-html-parser: a style element holding the text "<b>x</b>"; ours: html:style holding a b element -->
```

### Equals sign before an attribute name

Ours: `<a =b>` is an attribute named `=b`, as in browsers.
`angular-html-parser` reads an empty name with the value `b`.
`runner: = starts an attribute name`

### Raw text end tag

Ours: raw text closes only at `</` followed right away by its name, as in the HTML tokenizer and Vue, so `</ script>` inside a script is content.
`angular-html-parser` allows whitespace between `</` and the name of the end tag that closes raw text (`</ script>`), as it does in any end tag.

```html
<script>const s = "</ script>";</script>
<!-- angular-html-parser: the script closes at </ script>, then an unexpected </script> error; ours (browsers, Vue): one script -->
```

The end tag then runs to its first `>`, as any end tag does (`</script foo>` closes the script).
`runner: raw text end after whitespace`

### Abruptly closed empty comment

Ours: `<!-->` and `<!--->` are empty comments, as in the HTML tokenizer and Vue.
`angular-html-parser` reads on to the next `-->`.

```html
<!-->a-->
<!-- angular-html-parser: one comment holding ">a"; ours (browsers, Vue): an empty comment, then the text "a-->" -->
```
`runner: empty comment`

### End tag with attributes

Ours: an end tag runs to its first `>`, as in the HTML tokenizer (and Vue): what follows the name is ignored, and `</div foo>` closes the `div`.
`angular-html-parser` reports it.

## HTML

### Self-closing non-void elements

Ours: `/>` on a non-void HTML element is ignored, as in browsers (they keep it on void and foreign, SVG / MathML, elements).
The written `/>` is kept as `Element::self_closing`.
Prettier passes `canSelfClose: true`, so every element self-closes.

```html
<div />after</div>
<!-- Prettier: <div/> then text "after", then an unexpected </div> error -->
<!-- ours: div containing "after" -->
```
`runner: /> on non-void`

### Elements left open by an ignored self-close

Ours: an ancestor's end tag closes them, as in browsers.
`angular-html-parser` lets an ancestor's end tag close only the `closedByParent` set (`p`, `li`, `td`, ...).
Since the previous entry leaves `<my-el />` open, that set alone would make the common shape below an error.

```html
<div><my-el /></div>
<!-- ours: my-el closes at </div>, without an end tag -->
```

### Bracketed attribute names

Ours: in Angular only, a name opened with `[` runs while its brackets are open, `>` included, as in `angular-html-parser`;
in HTML the `>` ends the tag, as in browsers.
Angular templates rely on it for Tailwind's arbitrary variants (`[class.[&>svg]:w-4]`).
`runner: bracketed attribute name`

### Implicit end tags

Ours: WHATWG's optional end tags, since browsers build an HTML page from its source.
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

### Blocks without a body

Ours: a block without a body ends at its parameters, as every other span ends at the construct's last delimiter (`>`, `{`, `;`, `)`);
the whitespace after it is a text.
`angular-html-parser` reads the whitespace after the parameters into a block's start (it may precede the `{`),
so a fallthrough `@case` takes the whitespace up to the next `@case` into its span.

```html
@switch (a) { @case (1) @case (2) {x} }
<!-- angular-html-parser: "@case (1) ", then "@case (2) {x}"; ours: "@case (1)", the text " ", then "@case (2) {x}" -->
```
`runner: fallthrough case`

### Unterminated interpolation

Ours: an error, as in Vue; a tag ends the search even inside a string, as in Angular.
Angular reads a `{{` without its `}}` before the next tag or the end of the input as text
(its lexer: "This is actually wrong but here for backward compatibility").
No template relies on it (Angular escapes a literal `{{` as `{{ '{{' }}`).
`runner: unterminated interpolation`

## Vue

### Interpolation end

Ours: `{{ }}` closes at the first `}}`, tags included, as Vue's tokenizer only looks for `}}` (`{{ '}}' }}` closes inside the string, as in Vue).
`angular-html-parser` tracks quotes inside interpolation and ends it at a tag start,
so `{{ a<b }}` is an error in Prettier.

### Implicit closing and end tag case

Ours: no implicit end tags (`<p><div></div></p>` nests), and end tags match case-insensitively (`</transition>` closes `<Transition>`), as in Vue.
Prettier's Vue options keep the HTML implicit-close table and match case-sensitively.

### `v-pre`

Ours: inside `v-pre`, `{{ }}` is text and directive names are plain attributes,
including the `v-pre` element's own directives, written before `v-pre` or after, as in Vue.
Prettier formats both.

### HTML pages

Ours: a root `<html>` or `<!doctype html>` makes the file an HTML page, not an SFC, as Prettier reads a Vue file:
it parses with HTML rules (`{{ }}` and directives stay Vue).
Vue reads such a root element as a custom block holding raw text.
`runner: html page`

### Directive names without an argument

Ours: `v-slot:`, `v-bind:` and a lone `:` / `@` / `#` are plain attributes: a broken name keeps its source as written, with no part to format.
Vue reads them as directives with an empty argument.
`runner: directive without argument`

### Bogus comments

Ours: kept as comments, as all content: what `<!` or `<?` opens without a comment or CDATA is a bogus comment up to its `>`,
as in the HTML tokenizer (`<!x>`, `<?x?>`, `<!- c -->`).
Vue drops it, reads some as text (`<!- c -->`, `<! />`), and reads `<!>` on to a later `>`.
`run.ts` drops the ones Vue drops from our signature.
`runner: bogus comment`

### Slash in a start tag

Ours: a `/` that no `>` follows is an error (`<a / b>`, `<C a="b"/ c>`), as in `angular-html-parser`.
Vue and browsers ignore the `/`.
`runner: / in start tag`

### `br` end tag

Ours: `</br>` is an error, as in `angular-html-parser`.
Vue reads it as `<br>`, as HTML tree construction does, which we do not do.
`runner: </br>`

### Interpolation in RCDATA

Ours: in `<title>` and `<textarea>`, `<{{ t }}` holds the text `<` and an interpolation, as anywhere else in them,
and an interpolation without its `}}` before the end tag is an error.
Vue's tokenizer misses an interpolation right after a `<` there, and reads an unterminated one on past the end tag.
`runner: {{ }} in RCDATA`

## Kept: the oracles over the HTML tokenizer

Not divergences: recorded so that they are not reopened.
Svelte's and Astro's readings are noted for the dialects to come.

### Raw text elements that hold markup

Ours: `xmp`, `iframe`, `noembed`, `noframes`, `noscript` and `plaintext` hold markup, as the oracles read them.
Browsers read their content as raw text (`noscript` only with scripting on).
None of them holds markup in valid HTML (obsolete, or nothing allowed) but `noscript`, whose markup is what a printer formats.
Svelte reads markup too; Astro, raw text.

### CDATA in HTML content

Ours: the section is kept whole, as `angular-html-parser` (and Angular's compiler) does.
Browsers read `<![CDATA[a>b]]>` outside SVG and MathML as a bogus comment up to the first `>`, then text:
the cut would leave the rest as text a printer re-spaces.
Vue and Svelte report it.

### Duplicate attributes

Ours: kept, as in `angular-html-parser` and Astro.
Browsers drop all but the first, which would drop source.
Vue and Svelte report them.
