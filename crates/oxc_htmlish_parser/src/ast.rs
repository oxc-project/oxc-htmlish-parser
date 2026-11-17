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
    Interpolation(Interpolation<'a>),
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
    /// `[x]`, its value read as an expression; `None` without a value, and where Angular reports an error its tokens show.
    Property(Option<ArenaBox<'a, AngularExpression<'a>>>),
    /// `(x)`, its value read as an action; `None` without a value, and where Angular reports an error its tokens show.
    Event(Option<ArenaBox<'a, AngularAction<'a>>>),
    /// `[(x)]`, its value read as an expression, as `[x]`'s.
    TwoWay(Option<ArenaBox<'a, AngularExpression<'a>>>),
    /// `*x`: a structural directive, its value split as its microsyntax ([`TemplateBinding`]);
    /// `None` without a value, and where Angular reports one.
    /// Split as written: Angular splits the value with its entities decoded, so `&#59;` is a `;` there, but not here.
    Template(Option<ArenaBox<'a, ArenaVec<'a, TemplateBinding>>>),
    /// `i18n`, or `i18n-x`, which marks the attribute `x` for translation.
    I18n,
    /// Another name with `{{ }}` in its value (`title="Hi {{ name }}"`), which Angular binds:
    /// each `{{ }}`, read as in text.
    Interpolation(ArenaBox<'a, ArenaVec<'a, ValueInterpolation<'a>>>),
}

impl AngularAttribute<'_> {
    /// The part of `name` (the attribute's) between the delimiters: `x` of `[(x)]`, `i18n-x` and `*x`; empty for `i18n`.
    pub fn target(&self, name: Span) -> Span {
        let (start, end) = match self {
            Self::Property(_) | Self::Event(_) => (1, 1),
            Self::TwoWay(_) => (2, 2),
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

#[derive(Debug)]
pub struct Interpolation<'a> {
    /// Angular's reading of the expression; `None` in Vue, and where Angular reports an error its tokens show.
    pub angular: Option<ArenaBox<'a, AngularExpression<'a>>>,
}

impl Interpolation<'_> {
    /// Between `{{` and `}}`, of an interpolation node's `span`.
    pub fn expression(span: Span) -> Span {
        Span::new(span.start + 2, span.end - 2)
    }
}

/// An Angular expression (`{{ }}`, a property binding's value), as its compiler reads it (`parsePipe`).
///
/// The pipes at its root split off, and the comment it ends in; each span is from the first token to the last.
/// Read as written: Angular reads it with its character references decoded, so `&quot;` is a `"` there, but not here.
#[derive(Debug)]
pub struct AngularExpression<'a> {
    /// The code before the first pipe at the root, and before the comment.
    /// A pipe not at the root stays in it: in brackets, a template literal's `${}`, a conditional's branch ([`AngularCode::conditional`]).
    pub code: AngularCode,
    pub pipes: ArenaVec<'a, Pipe<'a>>,
    /// `//` to the end, its trailing whitespace left out: Angular reads no code past it, line breaks included.
    pub comment: Option<Span>,
}

/// A `{{ }}` in an attribute's value ([`AngularAttribute::Interpolation`]).
#[derive(Debug)]
pub struct ValueInterpolation<'a> {
    /// Between `{{` and `}}`.
    pub expression: Span,
    /// Angular's reading of it; `None` where it reports an error its tokens show.
    pub angular: Option<ArenaBox<'a, AngularExpression<'a>>>,
}

/// An Angular action (an event binding's value), as its compiler reads it (`parseAction`):
/// its statements, between the `;` at the root, and the comment it ends in. Read as written, as [`AngularExpression`].
#[derive(Debug)]
pub struct AngularAction<'a> {
    /// Empty ones left out (`a;;b`, a trailing `;`), which Angular reads as none.
    pub statements: ArenaVec<'a, AngularCode>,
    /// `//` to the end, its trailing whitespace left out.
    pub comment: Option<Span>,
}

/// `| name: argument : argument` of an [`AngularExpression`].
#[derive(Debug)]
pub struct Pipe<'a> {
    pub name: Span,
    pub arguments: ArenaVec<'a, AngularCode>,
}

/// A piece of code of an [`AngularExpression`]: its code, or a pipe's argument.
#[derive(Clone, Copy, Debug)]
pub struct AngularCode {
    pub span: Span,
    /// Its root is a conditional, parens around all of it aside (`a ? b : c`, `(a ? b : c)`):
    /// a pipe after it is its branch's unless it is in parens (`a ? b : c | p` is `a ? b : (c | p)`).
    pub conditional: bool,
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
    /// The name as Angular's compiler reads it.
    pub kind: BlockKind,
    /// Between the parentheses, split at `;` outside quotes and parentheses.
    /// Empty without parentheses.
    pub parameters: ArenaVec<'a, BlockParameter<'a>>,
    /// From `@` through the `{`, or through the parameters (or `;`) for a block without a body.
    pub open: Span,
    /// The `}`; `None` for a block without a body
    /// (`@case (a)` right before another case, `@default never;`).
    pub close: Option<Span>,
}

/// An Angular block's name, as its compiler reads it: the words of `else if` and `default never` are apart by
/// whitespace that its `/^else[^\S\r\n]+if/` takes (NBSP too), not a line break or U+000E–U+001F.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    If,
    ElseIf,
    Else,
    For,
    Empty,
    Switch,
    Case,
    Default,
    /// `default never`, which may go without a body (`@default never;`; then [`Block::close`] is `None`).
    DefaultNever,
    Defer,
    Placeholder,
    Loading,
    Error,
    Boundary,
    Content,
    /// A name it reports (`@iffy`, `@else⏎if`, `@else iffy`; DIVERGENCES.md#block-names-read-by-prefix).
    Unknown,
}

impl BlockKind {
    /// Every kind but [`Unknown`](Self::Unknown).
    pub const KNOWN: [Self; 15] = [
        Self::If,
        Self::ElseIf,
        Self::Else,
        Self::For,
        Self::Empty,
        Self::Switch,
        Self::Case,
        Self::Default,
        Self::DefaultNever,
        Self::Defer,
        Self::Placeholder,
        Self::Loading,
        Self::Error,
        Self::Boundary,
        Self::Content,
    ];

    /// The name with one space between its words; `None` for [`Unknown`](Self::Unknown).
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            Self::If => "if",
            Self::ElseIf => "else if",
            Self::Else => "else",
            Self::For => "for",
            Self::Empty => "empty",
            Self::Switch => "switch",
            Self::Case => "case",
            Self::Default => "default",
            Self::DefaultNever => "default never",
            Self::Defer => "defer",
            Self::Placeholder => "placeholder",
            Self::Loading => "loading",
            Self::Error => "error",
            Self::Boundary => "boundary",
            Self::Content => "content",
            Self::Unknown => return None,
        })
    }
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
