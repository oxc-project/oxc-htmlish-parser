//! AST node definitions.
//!
//! - Every node lives in one list ([`Root::nodes`]), linked by [`NodeId`]s to its parent, next sibling and children:
//!   a formatter walks up and down and keys its own tables by `NodeId`.
//! - No cooked values: names, attribute values and text are [`Span`]s into the original source.
//!   Entities are never decoded.
//! - Facts a printer would otherwise re-derive from the source or the tag table
//!   (whitespace around text, how a tag was written, void and raw text elements, directive parts) are fields.
//! - Formatter policy (CSS display, whitespace sensitivity) stays out.
//!
//! Dialect constructs that are not parsed yet slot in as new [`NodeKind`] / [`AttributeKind`] variants:
//! Angular `@let` and ICU;
//! Svelte `{expr}` tags and blocks;
//! Astro `{expr}` holding htmlish chunks.

use std::num::NonZeroU32;

use oxc_allocator::{ArenaBox, ArenaVec};

use crate::span::Span;

/// The parsed document.
#[derive(Debug)]
pub struct Root<'a> {
    /// Every node, in document order; the first is the document itself ([`NodeId::DOCUMENT`]).
    pub nodes: ArenaVec<'a, Node<'a>>,
    /// Vue only: the root elements are SFC blocks.
    /// `false` when the file was read as plain HTML (a root `<html>` or `<!doctype html>`).
    pub sfc: bool,
}

impl<'a> Root<'a> {
    /// The children of `id`, in order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = (NodeId, &Node<'a>)> {
        std::iter::successors(self[id].first_child, |&child| self[child].next)
            .map(|child| (child, &self[child]))
    }
}

impl<'a> std::ops::Index<NodeId> for Root<'a> {
    type Output = Node<'a>;

    fn index(&self, id: NodeId) -> &Node<'a> {
        &self.nodes[id.index()]
    }
}

/// A node's index in [`Root::nodes`]; `Option<NodeId>` is 4 bytes.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    pub const DOCUMENT: Self = Self(NonZeroU32::MIN);

    /// # Panics
    /// When `index` does not fit in `u32` (the parser rejects sources that large first).
    pub fn from_index(index: usize) -> Self {
        Self(NonZeroU32::new(u32::try_from(index + 1).unwrap()).unwrap())
    }

    pub fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

#[derive(Debug)]
pub struct Node<'a> {
    pub kind: NodeKind<'a>,
    pub span: Span,
    /// `None` for the document only.
    pub parent: Option<NodeId>,
    pub next: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
}

#[derive(Debug)]
pub enum NodeKind<'a> {
    Document,
    Element(ArenaBox<'a, Element<'a>>),
    /// Character data between markup, never empty. Whitespace-only text is kept.
    Text(Text),
    /// Vue `{{ }}`.
    Interpolation(Interpolation),
    Comment(Comment),
    /// `<!--[if IE]> … <![endif]-->`; its content is the node's children.
    ConditionalComment(ArenaBox<'a, ConditionalComment>),
    /// `<!--[if IE]><!-->`.
    ConditionalStart(ConditionalStart),
    /// `<!--<![endif]-->`.
    ConditionalEnd,
    DocType(DocType),
    Cdata,
    /// An Angular control flow block (`@if (a) { … }`); its content is the node's children.
    Block(ArenaBox<'a, Block<'a>>),
}

#[derive(Debug)]
pub struct Element<'a> {
    /// The tag name as written.
    pub name: Span,
    /// The name the tag table is keyed by: lowercased in HTML, as written in Vue.
    pub key: &'a str,
    pub namespace: Namespace,
    pub attributes: ArenaVec<'a, Attribute<'a>>,
    /// `<name …>`, including the `/>` when written.
    pub start_tag: Span,
    /// The start tag ends with `/>`.
    /// In HTML this closes only void and foreign elements; elsewhere the `/` is ignored and the element has children.
    pub self_closing: bool,
    /// A void element: never has children or an end tag.
    pub void: bool,
    /// How the content between the tags was read.
    pub content: Content,
    /// HTML drops a line break right after the start tag (`pre` / `textarea` / `listing`, in the HTML namespace).
    pub ignores_first_lf: bool,
    /// That line break was there; it is not part of any child.
    pub leading_lf: bool,
    /// The end tag as written.
    /// `None` for void and self-closed elements, and for elements closed implicitly or by EOF.
    pub end_tag: Option<EndTag>,
}

#[derive(Clone, Copy, Debug)]
pub struct EndTag {
    /// `</name>`.
    pub span: Span,
    /// The name as written, which may differ in case from the start tag's; the `/` of htm's `<//>`.
    pub name: Span,
    /// What follows the name before the `>`, trimmed, which HTML ignores (`foo` of `</div foo>`).
    pub ignored: Option<Span>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Content {
    /// Markup.
    #[default]
    Normal,
    /// No markup until the end tag: `script`, `style`, a Vue SFC block.
    Raw,
    /// No markup until the end tag, but Vue `{{ }}` still splits: `textarea`, `title`.
    Escapable,
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
/// Malformed names (`v-` alone, an unclosed `[`) stay [`AttributeKind::Plain`].
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

#[derive(Clone, Copy, Debug)]
pub struct Text {
    /// The node's span without leading and trailing whitespace (HTML ASCII whitespace; NBSP is content).
    /// Empty (at the span's end) for whitespace-only text.
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
    /// Between `{{` and `}}`.
    pub expression: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct Comment {
    /// Between `<!--` and `-->`, or the bogus comment's data.
    pub content: Span,
    /// `<!x>` / `<?x>`: not a real comment, printed verbatim.
    pub bogus: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ConditionalComment {
    /// Between `[if` and `]`.
    pub condition: Span,
    /// Between `]>` and `<![endif]`.
    pub content: Span,
    /// `content` parses on its own; otherwise there are no children.
    pub parsed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ConditionalStart {
    /// Between `[if` and `]`.
    pub condition: Span,
}

#[derive(Debug)]
pub struct Block<'a> {
    /// After `@`, as written (`if`, `else if`, `else   if`).
    pub name: Span,
    /// Between the parentheses, split at `;` outside quotes and parentheses, each trimmed.
    /// Empty without parentheses.
    pub parameters: ArenaVec<'a, Span>,
    /// From `@` through the `{`, or through the parameters (or `;`) for a block without a body.
    pub open: Span,
    /// The `}`; `None` for a block without a body
    /// (`@case (a)` right before another case, `@default never;`).
    pub close: Option<Span>,
}

#[derive(Clone, Copy, Debug)]
pub struct DocType {
    /// After `<!doctype`, trimmed (`html`).
    pub value: Span,
}
