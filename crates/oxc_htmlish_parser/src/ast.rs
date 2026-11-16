//! AST node definitions.
//!
//! - Every node lives in one list ([`Root::nodes`]), linked by [`NodeId`]s to its parent, next sibling and children:
//!   a formatter walks up and down and keys its own tables by `NodeId`.
//! - No cooked values: names, attribute values and text are [`Span`]s into the original source.
//!   Entities are never decoded.
//! - Facts a printer would otherwise re-derive from the source or the tag table
//!   (whitespace around text, how a tag was written, void and raw text elements, directive parts and binding kinds,
//!   `v-for`'s, `@for`'s and a microsyntax's splits) are fields; spans that follow from others are methods
//!   ([`Attribute::span`], [`AngularAttribute::target`]).
//! - Formatter policy (CSS display, whitespace sensitivity) stays out.
//!
//! Dialect constructs that are not parsed yet slot in as new [`NodeKind`] / [`AttributeKind`] variants:
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
    /// `{{ }}` (Vue, Angular).
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
    /// `@let name = value;` (Angular).
    LetDeclaration(ArenaBox<'a, LetDeclaration>),
    /// An Angular ICU expression (`{n, plural, =0 {none} other {many}}`); its cases are the node's children.
    Icu(ArenaBox<'a, Icu>),
    /// A case of an [`NodeKind::Icu`] (`=0 {none}`); its content is the node's children.
    IcuCase(IcuCase),
    /// The end tag of an `html`, `head` or `body` whose start tag is omitted (`<title>t</title></head>`):
    /// HTML inserts the element around the content, which stays where it is; the end tag follows it.
    EndTag(ArenaBox<'a, EndTag>),
}

#[derive(Debug)]
pub struct Element<'a> {
    /// The tag name as written.
    pub name: Span,
    /// The name the tag table is keyed by: lowercased in HTML and Angular, as written in a Vue SFC;
    /// in Angular without its namespace prefix (`svg:rect` is `rect`).
    pub key: &'a str,
    pub namespace: Namespace,
    /// In order, Angular's start-tag comments among them ([`AttributeKind::Comment`]).
    pub attributes: ArenaVec<'a, Attribute<'a>>,
    /// `<name …>`, including the `/>` when written; in a Vue SFC, a start tag a `</` ends has no `>` (`<C a="b"</C>`).
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
    /// No markup until the end tag, but `{{ }}` still splits (Vue, Angular): `textarea`, `title`.
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
}

impl Attribute<'_> {
    /// From the name through the value's closing quote.
    pub fn span(&self) -> Span {
        let end = self
            .value
            .map_or(self.name.end, |value| value.span.end + u32::from(value.quote != Quote::None));
        Span::new(self.name.start, end)
    }
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
    /// An Angular start-tag comment, among the attributes as written: [`Attribute::name`] is all of it, and there is no value.
    /// `//` up to the line break, its trailing whitespace left out (the `>` it runs past is in it: `<div // c>`), or `/*` through `*/`.
    Comment,
    Angular(AngularAttribute<'a>),
}

/// An Angular binding or i18n marker; the part of the name it binds is [`target`](Self::target).
///
/// Read as the compiler's `parseAttribute` tells a binding from a plain attribute.
/// Its other prefixes stay [`AttributeKind::Plain`] (`#ref`, `let-x`, `@animation`, and the old `bind-`, `on-`, `bindon-`, `ref-`),
/// and so do the delimiters around an empty name (`[]`) or that do not close (`[(x]`).
#[derive(Debug)]
pub enum AngularAttribute<'a> {
    /// `[x]`.
    Property,
    /// `(x)`.
    Event,
    /// `[(x)]`.
    TwoWay,
    /// `*x`: a structural directive, its value split as its microsyntax ([`TemplateBinding`]);
    /// `None` without a value, and where Angular reports one.
    /// Split as written: Angular splits the value with its entities decoded, so `&#59;` is a `;` there, but not here.
    Template(Option<ArenaBox<'a, ArenaVec<'a, TemplateBinding>>>),
    /// `i18n`, or `i18n-x`, which marks the attribute `x` for translation.
    I18n,
    /// Another name with `{{ }}` in its value (`title="Hi {{ name }}"`), which Angular binds:
    /// the expressions, between each `{{` and `}}`, read as in text.
    Interpolation(ArenaBox<'a, ArenaVec<'a, Span>>),
}

impl AngularAttribute<'_> {
    /// The part of `name` (the attribute's) between the delimiters: `x` of `[(x)]`, `i18n-x` and `*x`; empty for `i18n`.
    pub fn target(&self, name: Span) -> Span {
        let (start, end) = match self {
            Self::Property | Self::Event => (1, 1),
            Self::TwoWay => (2, 2),
            Self::Template(_) => (1, 0),
            Self::Interpolation(_) => (0, 0),
            Self::I18n => ((name.end - name.start).min(5), 0),
        };
        Span::new(name.start + start, name.end - end)
    }
}

/// A binding of a structural directive's microsyntax (`*ngFor="let item of items; trackBy: f"`).
///
/// Read as Angular's compiler reads it (`parseTemplateBindings`); each span inside [`Attribute::value`].
/// A key is identifiers, keywords or strings joined by `-`, as written; the `:` and the `;` or `,` after a binding are optional.
#[derive(Clone, Copy, Debug)]
pub enum TemplateBinding {
    /// `key: expression`, or the directive's own first (`key` `None`: `items` of `*ngIf="items"`);
    /// `expression` `None` when none follows (`*ngIf="as x"`).
    Expression { key: Option<Span>, expression: Option<Span> },
    /// `let name` / `let name = value`.
    Let { name: Span, value: Option<Span> },
    /// `as alias`, right after the [`Expression`](Self::Expression) whose key it names (`items | async as list`),
    /// or after a key of its own (`index as i`).
    As { key: Option<Span>, alias: Span },
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
    /// Always empty for slots (`#a.b` / `v-slot:a.b`): Vue reads the dots as part of the argument,
    /// and drops them without one (`v-slot.a`, `#.a`).
    pub modifiers: ArenaVec<'a, Span>,
    /// `v-for`'s value, split; `None` for another directive, a `v-for` without a value,
    /// or a value Vue does not read as `aliases in source`.
    pub for_expression: Option<ArenaBox<'a, VueFor>>,
}

/// `v-for`'s value split as Vue's `parseForExpression` does, each part trimmed, spans inside [`Attribute::value`].
///
/// Split as written: Vue splits the value with its entities decoded, so `&#44;` is a comma there, but not here.
#[derive(Clone, Copy, Debug)]
pub struct VueFor {
    /// What comes before the operator, without the parens around it (`item, index` of `(item, index) in items`).
    pub aliases: Span,
    /// The aliases Vue reads in `aliases`, split at its last one or two commas with no `]` / `}` after them.
    /// Each is `None` when empty (`, i`).
    pub value: Option<Span>,
    pub key: Option<Span>,
    pub index: Option<Span>,
    /// `in` or `of`.
    pub operator: Span,
    pub source: Span,
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
    /// Between the parentheses, split at `;` outside quotes and parentheses.
    /// Empty without parentheses.
    pub parameters: ArenaVec<'a, BlockParameter<'a>>,
    /// From `@` through the `{`, or through the parameters (or `;`) for a block without a body.
    pub open: Span,
    /// The `}`; `None` for a block without a body
    /// (`@case (a)` right before another case, `@default never;`).
    pub close: Option<Span>,
}

#[derive(Debug)]
pub struct BlockParameter<'a> {
    /// Trimmed.
    pub span: Span,
    pub kind: BlockParameterKind<'a>,
}

/// A parameter of `@for`, `@if` or `@else if`, split as Angular's compiler reads it
/// (`parseForLoopParameters`, `parseConditionalBlockParameters`), each part trimmed and inside [`BlockParameter::span`].
///
/// [`Plain`](Self::Plain) for the other blocks' parameters, the first of `@if`, and a shape the compiler reports;
/// the checks it makes of the names (`$index`, duplicates) are not repeated.
#[derive(Debug)]
pub enum BlockParameterKind<'a> {
    Plain,
    /// `@for`'s first: `item of expression`, inside the parentheses Angular strips (`(item of items)`).
    ForOf {
        item: Span,
        expression: Span,
    },
    /// `@for`'s `track expression`.
    Track {
        expression: Span,
    },
    /// `@for`'s `let name = variable, …`.
    Let(ArenaVec<'a, LetVariable>),
    /// `@if`'s and `@else if`'s `as alias`.
    As {
        alias: Span,
    },
}

/// `i = $index` of `@for`'s `let i = $index, e = $even`.
#[derive(Clone, Copy, Debug)]
pub struct LetVariable {
    pub name: Span,
    pub value: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct LetDeclaration {
    /// After `@let` and whitespace; empty when none is written (`@let = 1;`).
    pub name: Span,
    /// Between `=` and `;`, trimmed.
    pub value: Span,
}

/// The whitespace between the cases is no node, as in Angular's AST;
/// at the start of a case it is text, which Angular's lexer skips.
#[derive(Clone, Copy, Debug)]
pub struct Icu {
    /// Before the first `,`, trimmed (`n`).
    pub expression: Span,
    /// Between the `,`s, trimmed (`plural`, `select`).
    pub type_name: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct IcuCase {
    /// Before the `{`, trimmed (`=0`, `other`).
    pub value: Span,
    /// The offset of its `{`; the `}` ends the node's span.
    pub open: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct DocType {
    /// After `<!doctype`, trimmed (`html`).
    pub value: Span,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Node<'static>>() == 40);
    assert!(size_of::<NodeKind<'static>>() == 16);
    assert!(size_of::<Element<'static>>() == 96);
    assert!(size_of::<Attribute<'static>>() == 40);
};
