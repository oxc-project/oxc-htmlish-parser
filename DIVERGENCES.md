# Divergences from the oracles

The oracles and their norms are AGENTS.md's "The one rule":
`angular-html-parser` for html (WHATWG the norm), `@vue/compiler-dom`'s SFC parse for vue, `@angular/compiler` for angular.

Each entry is a place where we deliberately do not read as the oracle does: its heading names the concern, `Ours:` says what we do,
and the rest what the oracle does and why, with the input that shows it.
`decided by:` names the step of AGENTS.md's "Deciding a difference" that settled it, `Trees`, or `representation`.
`fixture:` names the fixtures under `crates/oxc_htmlish_parser/tests/fixtures/` that pin it.
`runner:` names the shape `tasks/differential/dialects.ts` exempts (some only in mutated inputs and files, where whole-tag tokens do not limit them);
entries without one are never generated there.
`cargo test` checks these lines (`tests/divergences.rs`).
Entries are grouped by the dialects they apply to.
The keeps that were weighed close the file.

## All dialects

### Name characters

Ours: `<` and `=` are part of tag names, attribute names and Vue directive arguments, as in the HTML tokenizer (quotes too, in tag names):
`<p=e>` is an element named `p=e`, `<a<C />` one named `a<C`, and `<div <span>` a `div` with an attribute named `<span`.
`angular-html-parser` reports these (or splits off an attribute without a name), and Vue a `<` in a plain attribute name.
Angular's compiler ends a tag name at `= < ' "` too, reading the rest as attributes or text (`<div=a=b>` is a `div`), or reports it;
printed as written, the name reads to Angular as it did.
No valid template uses them, and reading them as browsers do keeps every dialect on one rule.
`decided by: 3 for tag names in HTML (the tokenizer reads them without an error), 4 for the rest`
`fixture: html/markup.html`, `fixture: vue/template.vue`
`runner: punctuation in a name`

### End tag without a name

Ours: `</` without a name after it (past whitespace, in Vue and Angular) opens a bogus comment, as in the HTML tokenizer and Vue (`</3>`, `</=x>`); htm's `<//>` aside, in HTML.
`angular-html-parser` and Angular's compiler report it.
Vue's comment content skips whitespace after `</` (`</ 3>`); ours is the raw data.
`</` and whitespace before a name is HTML's "End tag after whitespace".
`decided by: 4`
`fixture: vue/bogus-and-foreign.vue`
`runner: </ whitespace`

### Script data escapes

Ours: a `<script` after `<!--` in a script hides the next `</script>` (until `-->`), as in the HTML tokenizer (and Astro):

```html
<script><!-- document.write("<script src=a.js></script>"); --></script>
<!-- angular-html-parser, Vue, Angular: the script closes inside the string, then an unexpected </script> error; ours (browsers): one script -->
```

Valid HTML that Prettier errors on.
Vue's compiler reports any script in a template, and the shared reading follows browsers;
Angular reads a script as its compiler does, up to its first end tag.
Vue SFC blocks are the exception: Vue's SFC parser reads them, not a browser, and they end at their first end tag.
`decided by: 3 in HTML, 4 in Vue`
`fixture: html/raw-text.html`, `fixture: angular/compiler-readings.html`
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

Prettier prints the second as `src="a.png"`, another URL (`oxc_formatter_htmlish`'s DIVERGENCES.md#unquoted-value-with-slash).
Angular's compiler ends every unquoted value at `/`, so the only valid template with one is `<cmp a=b/>` (or `<cmp a=/>`), which depends on it
(the Angular team keeps that reading, angular/angular#68804); it ends one at `=` too (`<a b=c=d>`), and Angular reads both so.
At the other characters, or a `/` elsewhere, Angular reports it; ours keeps the shared reading.
`decided by: 1 in HTML (3 for the values angular-html-parser reports), 4 in Angular`
`fixture: html/self-closing.html`, `fixture: angular/unquoted-value-self-closing.html`
`runner: unquoted value with a quote or =`

### End tag case

Ours: end tags match case-insensitively, as in browsers and Vue.
`<DIV></div>` is an error in `angular-html-parser` and Angular's compiler (they compare names as written).
`decided by: 3 in HTML, 4 in Angular`
`fixture: html/basic.html`

### Character references

Ours: not validated; text and values are never decoded here, so they are plain text, as browsers read them (with a parse error).
`&unknown;` and malformed numeric references are errors in `angular-html-parser` and Angular's compiler.
`decided by: 4`
`fixture: html/basic.html`

### End tag with attributes

Ours: an end tag runs to its first `>`, as in the HTML tokenizer (and Vue): what follows the name is ignored, and `</div foo>` closes the `div`.
`angular-html-parser` and Angular's compiler report it.
`decided by: 4`
`fixture: html/markup.html`

### Void element end

Ours: a void element closes before anything that follows it.
`angular-html-parser` and Angular's compiler keep a void element on their stack until the next token.
A doctype right after it becomes its child,
and a start tag right after it cannot implicitly close the void element's parent.

```html
<th><img src=x><td>
<!-- angular-html-parser, Angular: td inside th; ours (browsers): th, td siblings -->
```

In Angular the output keeps the tags, so Angular nests them the same way again;
what a printer adds between them is whitespace-only text, which Angular drops.
`decided by: 2 in HTML (a token left on a stack), 4 in Angular`
`fixture: html/implicit-close.html`, `fixture: angular/implicit-close.html`
`runner: doctype after void`, `runner: implicit close after void`

## HTML

Vue and Angular read the shapes below as their compilers do.

### End tag after whitespace

Ours: `</` and whitespace open a bogus comment, as browsers read it: `<div>a</ div>b` is a `div` holding `a`, a comment and `b`.
`angular-html-parser` reads `</ div >` as an end tag, as Vue's and Angular's compilers do.
Prettier prints it `</div>`, which closes the `div` there (`oxc_formatter_htmlish`'s DIVERGENCES.md#end-tag-with-space-is-comment).
`decided by: 1`
`fixture: html/markup.html`, `fixture: vue/compiler-readings.vue`, `fixture: angular/compiler-readings.html`
`runner: </ whitespace`

### Incorrectly closed comment

Ours: a comment closes at `--!>` too, as in the HTML tokenizer (and Astro): `<!-- a --!> b -->` is the comment ` a `, then the text ` b -->`.
`angular-html-parser` reads on to the next `-->`, as Vue's and Angular's compilers do (and Svelte).
`decided by: 2 (the fork has no branch for it)`
`fixture: html/markup.html`, `fixture: vue/compiler-readings.vue`, `fixture: angular/compiler-readings.html`
`runner: --!>`

### Abruptly closed empty comment

Ours: `<!-->` and `<!--->` are empty comments, as in the HTML tokenizer and Vue.
`angular-html-parser` reads on to the next `-->`, as Angular's compiler does.

```html
<!-->a-->
<!-- angular-html-parser, Angular: one comment holding ">a"; ours (browsers, Vue): an empty comment, then the text "a-->" -->
```

`decided by: 2`
`fixture: html/markup.html`, `fixture: angular/compiler-readings.html`
`runner: empty comment`

### Raw text end tag

Ours: raw text closes only at `</` followed right away by its name, as in the HTML tokenizer and Vue, so `</ script>` inside a script is content.
`angular-html-parser` allows whitespace between `</` and the name of the end tag that closes raw text, as it does in any end tag, and so does Angular's compiler.

```html
<style>a</ style><style>b</style>
<!-- angular-html-parser, Angular: two styles; ours (browsers, Vue): one style holding "a</ style><style>b" -->
```

Prettier prints the first end tag `</style`, which closes the style there for browsers.
The end tag then runs to its first `>`, as any end tag does (`</script foo>` closes the script).
`decided by: 1`
`fixture: html/raw-text.html`, `fixture: angular/compiler-readings.html`
`runner: raw text end after whitespace`

### Equals sign before an attribute name

Ours: `<a =b>` is an attribute named `=b`, as in browsers.
`angular-html-parser` reads an empty name with the value `b`, as Angular's compiler does.
Prettier prints it `<a ="b">`, an attribute named `="b"` to browsers.
`decided by: 1`
`fixture: html/markup.html`
`runner: = starts an attribute name`

### Whitespace in tags

Ours: tab, LF, FF, CR and space, as the HTML tokenizer has it.
`angular-html-parser` also treats NBSP and the controls U+000E–U+001F as whitespace inside tags, as Angular's compiler does.
Prettier prints `<div␠class="x">` (`␠` a NBSP) as `<div class="x">`, a `div` to browsers, which read an element named `div␠class="x"`.
`decided by: 1`
`fixture: html/markup.html`, `fixture: angular/compiler-readings.html`

### `foreignObject` children

Ours: HTML, as in browsers and Angular's compiler.
`angular-html-parser` declares `preventNamespaceInheritance` on `foreignObject`,
but the case-insensitive lookup (`foreignobject`) misses the entry, so its children stay SVG.
Angular's compiler looks the exact name up first; the fork's case-insensitive lookup is the accident.
`decided by: 2`
`fixture: html/self-closing.html`

### SVG `title`

Ours: it holds markup, the declared intent, as browsers read it.
`angular-html-parser`'s tag table declares SVG `title` as parsable data (only the HTML `title` is escapable raw text),
but its tokenizer never applies it, as in Angular's compiler (but for an explicit `svg:` prefix, angular/angular#40259).

```html
<svg><title><b>t</b></title></svg>
<!-- angular-html-parser, Angular: title holds the text "<b>t</b>"; ours: title holds a b element -->
```

`decided by: 2`
`fixture: html/raw-text.html`, `fixture: angular/compiler-readings.html`
`runner: svg title`

### Namespace prefixes

Ours: a `prefix:` is part of the element name: `<html:style>` and `<svg:rect>` are unknown HTML elements, with markup inside, as in browsers.
`angular-html-parser` reads `<html:style>` as `style` in the HTML namespace and `<svg:rect>` as an SVG `rect`, as Angular's compiler does
(a component template outside `<svg>` writes SVG elements so).
Prettier lays them out as the elements the prefix names (`oxc_formatter_htmlish`'s DIVERGENCES.md#prefixed-script-style).

```html
<p><html:style><b>x</b></html:style></p>
<!-- angular-html-parser, Angular: a style element holding the text "<b>x</b>"; ours: html:style holding a b element -->
```

`decided by: 1`
`fixture: html/namespace-prefix.html`, `fixture: angular/compiler-readings.html`
`runner: namespace prefix`

### Self-closing non-void elements

Ours: `/>` on a non-void HTML element is ignored, as in browsers (they keep it on void and foreign, SVG / MathML, elements).
The written `/>` is kept as `Element::self_closing`.
Prettier passes `canSelfClose: true`, so every element self-closes.

```html
<div />after</div>
<!-- Prettier: <div/> then text "after", then an unexpected </div> error -->
<!-- ours: div containing "after" -->
```

Prettier re-indents what follows `<pre />`, which is `pre` content to browsers (`oxc_formatter_htmlish`'s DIVERGENCES.md#pre-content-without-end-tag).
`decided by: 1`
`fixture: html/self-closing.html`
`runner: /> on non-void`

### Elements left open by an ignored self-close

Ours: an ancestor's end tag closes them, as in browsers (with a parse error).
`angular-html-parser` lets an ancestor's end tag close only the `closedByParent` set (`p`, `li`, `td`, ...).
Since the previous entry leaves `<my-el />` open, that set alone would make the common shape below an error.

```html
<div><my-el /></div>
<!-- ours: my-el closes at </div>, without an end tag -->
```

`decided by: 4`
`fixture: html/self-closing.html`

### Bracketed attribute names

Ours: a `>` ends the tag, as in browsers; in Angular, a name opened with `[` runs while its brackets are open, `>` included, as its compiler reads it.
`angular-html-parser` reads Angular's bracketed names in HTML too; Prettier passes no option to turn it off.
Angular templates rely on it for Tailwind's arbitrary variants (`[class.[&>svg]:w-4]`).
`decided by: 2`
`fixture: html/markup.html`, `fixture: angular/bracketed-attribute.html`
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
(AGENTS.md "Trees"):

- `p` does not close at `details`, `dialog`, `figcaption`, `figure`, `menu` or `search`: `<p><figure></figure></p>` parses.
- `head` closes only at `<body>`, not at everything it cannot hold:
  real pages put elements there before an explicit `</head>` (a browser moves them to the body).

So `<p>a<figure>`, and `<head><title>t</title><p>` with both `</head>` and `<body>` omitted,
read the second element inside the first, though both are valid HTML.
Both are blocks to a browser either way, so the whitespace a printer adds between them renders nothing.
`decided by: 2`
`fixture: html/implicit-close.html`
`runner: WHATWG optional end tags`

## Angular

### Blocks without a body

Ours: a block without a body ends at its parameters, as every other span ends at the construct's last delimiter (`>`, `{`, `;`, `)`);
the whitespace after it is a text.
`angular-html-parser` and Angular's compiler read the whitespace after the parameters into a block's start (it may precede the `{`),
so a fallthrough `@case` takes the whitespace up to the next `@case` into its span.

```html
@switch (a) { @case (1) @case (2) {x} }
<!-- Angular: "@case (1) ", then "@case (2) {x}"; ours: "@case (1)", the text " ", then "@case (2) {x}" -->
```

`decided by: representation`
`fixture: angular/blocks.html`
`runner: fallthrough case`

### How Angular's AST holds a node

Ours: every node as written: a doctype, a CDATA section, a comment's raw content,
and what `<!` or `<?` opens without a comment or CDATA as a bogus comment up to its `>`, as in the HTML tokenizer (`<!x>`, `<?xml ?>`).
Angular's compiler drops doctypes, and reads `<!x>` as one; it reads CDATA and `<?xml ?>` as text, and trims a comment.
Printed as written, the output reads to Angular as the input did.
`decided by: representation`
`fixture: angular/compiler-readings.html`

### Self-closing native elements

Ours: `/>` closes any element.
Angular's compiler closes void, custom and foreign elements so, and reports the rest (`<div/>`):
ours takes its rule for custom elements to native ones, as Vue and Prettier (`canSelfClose: true`) read them too.
`decided by: 4 (the norm's rule for a nearby valid shape)`
`fixture: angular/compiler-readings.html`

## Vue

### HTML pages

Ours: a root `<html>` or `<!doctype html>` makes the file an HTML page, not an SFC, as Prettier reads a Vue file:
it parses with HTML rules (`{{ }}` and directives stay Vue).
Vue reads such a root element as a custom block holding raw text, which nothing compiles, so formatting the page changes no output.
`decided by: 4 (an HTML page reads as the html dialect does)`
`fixture: vue/html-page.vue`
`runner: html page`

### Directive names without an argument in `v-pre`

Ours: `v-bind:="p"` under `v-pre` is a plain attribute named `v-bind:`, as written.
Vue names it `v-bind`, without the colon, and drops its value; printed as written, the output reads to Vue as the input did.
`decided by: representation`
`fixture: vue/directives.vue`
`runner: directive without argument in v-pre`

### Bogus comments

Ours: kept as comments, as all content: what `<!` or `<?` opens without a comment or CDATA is a bogus comment up to its `>`,
as in the HTML tokenizer (`<!x>`, `<?x?>`, `<!- c -->`).
Vue drops it, reads some as text (`<!- c -->`, `<! />`), and reads `<!>` on to a later `>`.
Printed as written, the output reads to Vue as the input did.
`dialects.ts` reads the ones Vue drops as their source text in our signature.
`decided by: representation`
`fixture: vue/bogus-and-foreign.vue`, `fixture: vue/template.vue`
`runner: bogus comment`

### Void element end tags

Ours: an end tag of a void element (`</br>`) is text, kept as written: `a</br>b` is the text `a</br>b`.
Vue drops the tag and joins the text around it (`ab`).
Its characters stay in ours, where a printer breaks a line only at whitespace (in `</br >` too), which Vue condenses as before.
HTML and Angular report it (the last section).
`decided by: 3, representation`
`fixture: vue/compiler-readings.vue`
`runner: </br>`

## Kept: the oracles over the norm

Not divergences: places where the oracle reads against the norm and we keep its reading, recorded so that they are not reopened.
Svelte's and Astro's readings are noted for the dialects to come.

### Raw text elements that hold markup

Ours: `xmp`, `iframe`, `noembed`, `noframes`, `noscript` and `plaintext` hold markup, as the oracles read them.
Browsers read their content as raw text (`noscript` only with scripting on).
None of them holds markup in valid HTML (obsolete, or nothing allowed) but `noscript`, whose markup is what a printer formats.
What browsers read as their text renders nothing, or (`xmp`, `plaintext`) prints as written.
Svelte reads markup too; Astro, raw text.
`decided by: 2 (intent: Angular's tag table)`
`fixture: html/raw-text.html`

### CDATA in HTML content

Ours: the section is kept whole, as `angular-html-parser` (and Angular's compiler) does.
Browsers read `<![CDATA[a>b]]>` outside SVG and MathML as a bogus comment up to the first `>`, then text:
the cut would leave the rest as text a printer re-spaces.
Vue and Svelte report it.
`decided by: 2 (intent: explicit CDATA lexing, in Angular's compiler too)`
`fixture: html/markup.html`

### Duplicate attributes

Ours: kept, as in the oracles (and Astro).
Browsers drop all but the first, which would drop source; printed as written, the output drops the same ones.
Vue and Svelte report them.
`decided by: representation`
`fixture: html/markup.html`

### Quotes in attribute names

Ours: an error, as in the oracles (in Vue, inside a plain attribute name; one opening a name, and any in a directive name, are part of it, as Vue reads them).
The HTML tokenizer reads `<a b"c>` as an attribute named `b"c`, with a parse error.
Such a name is almost always a template's expression nesting the quotes of a value (`content="{{ f("-") }}"`, in Nunjucks or Askama files):
read as browsers do, the printer would re-space what follows the cut value, changing the expression.
Angular's bracketed names may hold quotes (DIVERGENCES.md#bracketed-attribute-names).
`decided by: 4 (the shared reading would let a printer alter a template's expression)`
`fixture: vue/compiler-readings.vue`

### Slash in a start tag

Ours: a `/` that no `>` follows is an error (`<a / b>`, `<C a="b"/ c>`), as in the oracles (Vue reports `Illegal '/' in tags.`).
Browsers ignore the `/`, with a parse error; reading it so would drop it from the output.
`decided by: 4 (the shared reading would drop the slash)`

### Raw text in foreign elements

Ours: in HTML and Angular, `script`, `style` and the other raw text elements hold raw text in SVG and MathML too, as the oracles read them
(Vue reads markup there, as we do).
Browsers read their content as markup there.
A printer formats it as the language the element names, which browsers read as the same text, or prints it as written.
`decided by: 2 (intent: Angular's tag table, which names no namespace)`
`fixture: html/raw-text.html`

### Void element end tags in HTML and Angular

Ours: `</br>` is an error, as in `angular-html-parser` and Angular's compiler.
Browsers read it as `<br>`, with a parse error, which is tree construction we do not do (AGENTS.md "Trees").
`decided by: 4 (reading it as a br element would print a tag that is not written)`

### Trees rebuilt on a parse error

Ours: the written nesting, as in the oracles (AGENTS.md "Trees");
an end tag that would close an element that cannot close implicitly is an error (the adoption agency's `<b><i>x</b>y</i>`).
Browsers rebuild it on a parse error: foster parenting, the adoption agency, nested `a`.

```html
x<table>t<tr><td>c</td></tr></table>
<!-- browsers: the text "xt", then the table; ours (the oracles): "x", then a table holding "t" -->
```

A printer working from the written nesting lays `t` out inside the table, as Prettier does;
the line breaks it adds there render as a space between `x` and `t` once a browser moves `t` out.
A template's source is no page a browser reads (`<table>{% for r in rows %}<tr>`), so the written nesting is formatted as is, best effort.
`decided by: Trees`
`fixture: html/implicit-close.html`
