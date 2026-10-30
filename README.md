# oxc-htmlish-parser

`oxc-htmlish-parser` parses HTML and HTML-based template languages (Vue, Angular) into a span-faithful typed AST, designed for building formatters.

How each dialect is read, and why, is in [AGENTS.md](https://github.com/oxc-project/oxc-htmlish-parser/blob/main/AGENTS.md).
Deliberate differences from each dialect's oracle are listed in [DIVERGENCES.md](https://github.com/oxc-project/oxc-htmlish-parser/blob/main/DIVERGENCES.md).

## Example

```rust
use oxc_htmlish_parser::{Allocator, Dialect, Parser};

let allocator = Allocator::default();
let root = Parser::new(&allocator, "<p>Hello</p>\n", Dialect::Html).parse().unwrap();
println!("{root:#?}");
```

## License

MIT

# [Sponsored By](https://oxc.rs/sponsor)

<p align="center">
  <a href="https://oxc.rs/sponsor">
    <img src="https://raw.githubusercontent.com/oxc-project/sponsors/main/sponsors.svg" alt="Our sponsors" />
  </a>
</p>
