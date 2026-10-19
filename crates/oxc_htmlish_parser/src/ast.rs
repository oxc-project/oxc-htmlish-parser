//! AST node definitions.
//!
//! - No cooked values: names, attribute values and text are [`Span`]s into the original source.
//!   Entities are never decoded.
//! - Facts a printer would otherwise re-derive from the source (whitespace around text,
//!   how a tag was written, directive parts) are fields.
//! - Formatter policy (CSS display, whitespace sensitivity) stays out.
//!
//! Dialect constructs that are not parsed yet slot in as new [`Node`] / [`AttributeKind`] variants:
//! Angular control flow blocks, `@let` and ICU;
//! Svelte `{expr}` tags and blocks;
//! Astro `{expr}` holding htmlish chunks.

use oxc_allocator::{ArenaBox, ArenaVec};

use crate::span::Span;

/// The document.
#[derive(Debug)]
pub struct Root<'a> {
    pub children: ArenaVec<'a, Node<'a>>,
    /// Vue only: the root elements are SFC blocks.
    /// `false` when the file was read as plain HTML (a root `<html>` or `<!doctype html>`).
    pub sfc: bool,
    pub span: Span,
}

#[derive(Debug)]
pub enum Node<'a> {
    Element(ArenaBox<'a, Element<'a>>),
    Text(Text),
    /// Vue `{{ }}`.
    Interpolation(Interpolation),
    Comment(Comment),
    /// `<!--[if IE]> … <![endif]-->`.
    ConditionalComment(ArenaBox<'a, ConditionalComment<'a>>),
    /// `<!--[if IE]><!-->`.
    ConditionalStart(ConditionalStart),
    /// `<!--<![endif]-->`.
    ConditionalEnd(ConditionalEnd),
    DocType(DocType),
    Cdata(Cdata),
}

impl Node<'_> {
    pub fn span(&self) -> Span {
        match self {
            Node::Element(n) => n.span,
            Node::Text(n) => n.span,
            Node::Interpolation(n) => n.span,
            Node::Comment(n) => n.span,
            Node::ConditionalComment(n) => n.span,
            Node::ConditionalStart(n) => n.span,
            Node::ConditionalEnd(n) => n.span,
            Node::DocType(n) => n.span,
            Node::Cdata(n) => n.span,
        }
    }
}

#[derive(Debug)]
pub struct Element<'a> {
    /// The tag name as written.
    pub name: Span,
    pub namespace: Namespace,
    pub attributes: ArenaVec<'a, Attribute<'a>>,
    pub children: ArenaVec<'a, Node<'a>>,
    /// `<name …>`, including the `/>` when written.
    pub start_tag: Span,
    /// The start tag ends with `/>`.
    /// In HTML this closes only void and foreign elements; elsewhere the `/` is ignored and the element has children.
    pub self_closing: bool,
    /// The line break right after the start tag that HTML ignores (`pre` / `textarea` / `listing`).
    /// It is not part of any child.
    pub leading_lf: bool,
    /// The end tag as written.
    /// `None` for void and self-closed elements, and for elements closed implicitly or by EOF.
    pub end_tag: Option<Span>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

#[derive(Debug)]
pub struct Attribute<'a> {
    pub name: Span,
    pub value: Option<AttributeValue>,
    pub kind: AttributeKind<'a>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct AttributeValue {
    /// The value without its quotes.
    pub span: Span,
    pub quote: Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quote {
    None,
    Single,
    Double,
}

#[derive(Debug)]
pub enum AttributeKind<'a> {
    Plain,
    Vue(ArenaBox<'a, VueDirective<'a>>),
}

/// A Vue directive name split into its parts, each a span inside [`Attribute::name`].
///
/// Malformed names (`:` alone, an unclosed `[`) stay [`AttributeKind::Plain`].
#[derive(Debug)]
pub struct VueDirective<'a> {
    /// The name after `v-` (`if`, `bind`), or the shorthand (`:` / `@` / `#` / `.`).
    pub name: Span,
    pub argument: Option<VueArgument>,
    /// Without the dots.
    /// Always empty for slots (`#a.b` / `v-slot:a.b`): Vue reads the dots as part of the argument.
    pub modifiers: ArenaVec<'a, Span>,
}

#[derive(Clone, Copy, Debug)]
pub struct VueArgument {
    pub span: Span,
    /// `[key]`, brackets included in `span`.
    pub dynamic: bool,
}

/// Character data between markup, never empty.
/// Whitespace-only text is kept.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub span: Span,
    /// `span` without leading and trailing whitespace (HTML ASCII whitespace; NBSP is content).
    /// Empty (at `span.end`) for whitespace-only text.
    pub content: Span,
    pub leading: Whitespace,
    pub trailing: Whitespace,
}

impl Text {
    pub fn is_whitespace_only(&self) -> bool {
        self.content.is_empty()
    }
}

/// A whitespace run at an edge of a [`Text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whitespace {
    None,
    /// No line break.
    Space,
    /// Contains a line break.
    Newline,
}

#[derive(Clone, Copy, Debug)]
pub struct Interpolation {
    pub span: Span,
    /// Between `{{` and `}}`.
    pub expression: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct Comment {
    pub span: Span,
    /// Between `<!--` and `-->`, or the bogus comment's data.
    pub content: Span,
    /// `<!x>` / `<?x>`: not a real comment, printed verbatim.
    pub bogus: bool,
}

#[derive(Debug)]
pub struct ConditionalComment<'a> {
    /// Between `[if` and `]`.
    pub condition: Span,
    /// Between `]>` and `<![endif]`.
    pub content: Span,
    /// `None` when `content` does not parse on its own.
    pub children: Option<ArenaVec<'a, Node<'a>>>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct ConditionalStart {
    /// Between `[if` and `]`.
    pub condition: Span,
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct ConditionalEnd {
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct DocType {
    pub span: Span,
    /// After `<!doctype`, trimmed (`html`).
    pub value: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct Cdata {
    pub span: Span,
}
