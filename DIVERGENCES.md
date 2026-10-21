# Divergences from the oracles

The parse-behavior oracles:

- html and angular: `angular-html-parser` 10.13, with the options Prettier passes for each
- vue: `@vue/compiler-core` (Prettier reads Vue through `angular-html-parser` too; where they differ, Vue wins)

Each entry is a place where we deliberately do not match, with the input that shows it.
Where the oracle errors and we parse, Prettier fails on the file and nothing can depend on it.
`runner:` names the shape `tasks/differential/run.ts` exempts (some only in mutated inputs and files, where whole-tag tokens do not limit them);
entries without one are never generated there, fixtures pin them.
Entries are grouped by the dialects they apply to.

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

After the name, only whitespace may precede the `>`: `</script foo>` is content, as in `angular-html-parser` (Vue reports an error there).
`runner: raw text end after whitespace`

### A comment may close right after its opening

`<!-->` and `<!--->` are empty comments in the HTML tokenizer and Vue; `angular-html-parser` reads on to the next `-->`.

```html
<!-->a-->
<!-- angular-html-parser: one comment holding ">a"; ours (browsers, Vue): an empty comment, then the text "a-->" -->
```
`runner: empty comment`

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

## Vue

### `{{ }}` closes at the first `}}`, tags included

`angular-html-parser` tracks quotes inside interpolation and ends it at a tag start,
so `{{ a<b }}` is an error in Prettier.
Vue's tokenizer only looks for `}}`; so do we (`{{ '}}' }}` closes inside the string, as in Vue).

### No implicit closing, case-insensitive end tags

Vue has no implicit end tags (`<p><div></div></p>` nests), and matches `</transition>` to `<Transition>`.
Prettier's Vue options keep the HTML implicit-close table and match case-sensitively.

### A `<` inside a start tag is an error

Vue reads a `<` in a tag name as part of the name, as the HTML tokenizer does: `<a<C />` is an element named `a<C`.
No element or component has such a name; the shape is a start tag left unterminated before the next one.
We read it so in every dialect, rather than follow a tokenizer by-product no valid template uses (AGENTS.md "The one rule").
Vue already rejects a `<` where an attribute name starts or inside one.
`runner: < in start tag`

### `</` without a name is a bogus comment

As in Vue and browsers (`</ x>`, `</3>`).
Vue's comment content skips the whitespace after `</`; ours is the raw data.
`runner: </ whitespace`

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

Vue drops what `<!` or `<?` opens without a comment or CDATA (`<!x>`, `<?x?>`); we keep it as a comment, as all content.
`run.ts` drops them from our signature.
