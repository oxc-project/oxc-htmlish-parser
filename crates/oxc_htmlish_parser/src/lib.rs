//! oxc-htmlish-parser parses HTML and HTML-based template languages
//! into a span-faithful typed AST, designed for building formatters.
//!
//! It is not a spec-compliant HTML parser: there is no WHATWG tree construction,
//! only a tag table (void elements, raw text, implicit closing).
//! Dialects are a [`Dialect`] switch over one parser and one AST.
//!
//! ## Basic Usage
//!
//! ```rust
//! use oxc_htmlish_parser::{Allocator, Dialect, Parser};
//!
//! let allocator = Allocator::default();
//! let root = Parser::new(&allocator, "<p>Hello</p>\n", Dialect::Html).parse().unwrap();
//! assert_eq!(root.children.len(), 2);
//! ```

pub mod ast;
mod diagnostic;
mod parser;
mod span;
mod tags;
mod vue;

pub use diagnostic::{Diagnostic, DiagnosticKind};
pub use oxc_allocator::Allocator;
pub use parser::{Dialect, Parser};
pub use span::Span;
