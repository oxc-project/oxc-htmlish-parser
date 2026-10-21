# oxc-htmlish-parser

`oxc-htmlish-parser` parses HTML and HTML-based template languages into a span-faithful typed AST, designed for building formatters.

- Not a spec-compliant HTML parser: there is no WHATWG tree construction, only a tag table (void elements, raw text, implicit closing).
- Parse behavior targets each dialect's own parser (`angular-html-parser` for HTML and Angular, `@vue/compiler-core` for Vue).
  Deliberate differences are listed in [DIVERGENCES.md](./DIVERGENCES.md).
- Values are not cooked: consumers slice the original source through spans.
- Dialects: HTML, Vue, Angular (control flow blocks; ICU and `@let` are still text).

## Example

```rust
use oxc_htmlish_parser::{Allocator, Dialect, Parser};

let allocator = Allocator::default();
let root = Parser::new(&allocator, "<p>Hello</p>\n", Dialect::Html).parse().unwrap();
println!("{root:#?}");
```
