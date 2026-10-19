# Divergences from the oracles

The parse-behavior oracles:

- html: `angular-html-parser` 10.13, with the options Prettier passes
- vue: `@vue/compiler-core` (Prettier reads Vue through `angular-html-parser` too; where they differ, Vue wins)

Each entry is a place where we deliberately do not match, with the input that shows it.
Where the oracle errors and we parse, Prettier fails on the file and nothing can depend on it.
`runner:` names the shape `tasks/differential/run.ts` exempts; entries without one are never generated there, fixtures pin them.

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

### An element whose `/>` was ignored closes on an ancestor's end tag

`angular-html-parser` lets an ancestor's end tag close only the `closedByParent` set (`p`, `li`, `td`, ...).
Since the previous entry leaves `<my-el />` open, that set alone would make the common shape below an error;
browsers close it there.

```html
<div><my-el /></div>
<!-- ours: my-el closes at </div>, without an end tag -->
```

### Unquoted attribute values end at whitespace or `>` only

`angular-html-parser` also ends them at `/ ' " = <`.
Browsers and Vue do not.

```html
<a href=/foo/bar>x</a>
<!-- angular-html-parser: error -->
<img src=a.png/>
<!-- angular-html-parser: src="a.png", self-closed; ours (browsers): src="a.png/" -->
```

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

### An attribute name may start with `=`

`<a =b>` is an attribute named `=b`, as in browsers;
`angular-html-parser` reads an empty name with the value `b`.

## Vue

### `{{ }}` closes at the first `}}`, tags included

`angular-html-parser` tracks quotes inside interpolation and ends it at a tag start,
so `{{ a<b }}` is an error in Prettier.
Vue's tokenizer only looks for `}}`; so do we (`{{ '}}' }}` closes inside the string, as in Vue).

### No implicit closing, case-insensitive end tags

Vue has no implicit end tags (`<p><div></div></p>` nests), and matches `</transition>` to `<Transition>`.
Prettier's Vue options keep the HTML implicit-close table and match case-sensitively.

### A `<` inside a start tag is an error

Vue reads it as part of the tag name (`<a<C />` is an element named `a<C`); we reject it, as Prettier does.
`runner: < in start tag`

### `</` without a name is a bogus comment

As in Vue and browsers (`</ x>`, `</3>`).
Vue's comment content skips the whitespace after `</`; ours is the raw data.
`runner: </ whitespace`

### `v-pre`

Inside `v-pre`, `{{ }}` is text and directive names are plain attributes,
including the `v-pre` element's own directives, written before `v-pre` or after.
Prettier formats both.
