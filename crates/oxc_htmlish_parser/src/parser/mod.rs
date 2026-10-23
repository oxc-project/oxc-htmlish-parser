//! Parser entry point.
//!
//! One pass over the bytes builds the tree directly; there is no token stream.
//! Tree construction is limited to the tag table (`tags.rs`):
//! void elements, raw text, implicit closing.

use memchr::{memchr, memmem};

use oxc_allocator::{Allocator, ArenaVec};

use crate::{
    ast::*,
    diagnostic::{Diagnostic, DiagnosticKind},
    span::{Span, span},
};
use tags::Tag;

mod angular;
mod element;
mod markup;
mod tags;
mod text;
mod vue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Html,
    /// A Vue SFC.
    /// A file with a root `<html>` or `<!doctype html>` is read with HTML rules instead
    /// (`{{ }}` and directives are still Vue).
    Vue,
    /// An Angular template.
    /// HTML with `{{ }}`, control flow blocks (`@if (a) { … }`), and `/>` closing any element.
    /// TODO: ICU expressions and `@let` are not parsed yet (plain text).
    Angular,
}

pub struct Parser<'a> {
    allocator: &'a Allocator,
    source: &'a str,
    dialect: Dialect,
}

impl<'a> Parser<'a> {
    pub fn new(allocator: &'a Allocator, source: &'a str, dialect: Dialect) -> Self {
        Self { allocator, source, dialect }
    }

    /// Front matter is not handled here: callers strip it before parsing.
    ///
    /// # Errors
    ///
    /// The first broken structure (see [`DiagnosticKind`]); there is no recovery.
    pub fn parse(self) -> Result<Root<'a>, Diagnostic> {
        if u32::try_from(self.source.len()).is_err() {
            return Err(Diagnostic::new(DiagnosticKind::SourceTooLong, Span::empty(0)));
        }
        let mut rules = match self.dialect {
            Dialect::Html => Rules::HTML,
            Dialect::Vue => Rules::VUE,
            Dialect::Angular => Rules::ANGULAR,
        };
        // About one node per 16 bytes: the median of 994 real-world HTML and Angular templates (vscode, mdn-content, ecosystem-ci).
        // Vue SFCs are sparser (a `<script>` is one text node); reserving too much costs less than growing in the arena.
        let mut nodes = ArenaVec::with_capacity_in(self.source.len() / 16 + 1, &self.allocator);
        nodes.push(Node {
            kind: NodeKind::Document,
            span: span(0, self.source.len()),
            parent: None,
            next: None,
            first_child: None,
            last_child: None,
        });
        loop {
            let mut builder = Builder::new(self.allocator, self.source, rules, nodes);
            match builder.run() {
                Ok(()) => return Ok(Root { nodes: builder.nodes, sfc: rules.sfc }),
                Err(Stop::Error(diagnostic)) => return Err(diagnostic),
                Err(Stop::Html) => {
                    // Start over in the same buffer, keeping only the document node
                    nodes = builder.nodes;
                    nodes.truncate(1);
                    nodes[0].first_child = None;
                    nodes[0].last_child = None;
                    rules = Rules::VUE_HTML;
                }
            }
        }
    }
}

/// What a dialect switches on.
#[derive(Clone, Copy)]
struct Rules {
    /// `angular-html-parser`'s reading of tags rather than Vue's:
    /// case-insensitive tag table limited to the HTML namespace, implicit closing.
    html: bool,
    /// With `html`: WHATWG's optional end tags, which close through the ancestors whose end tag may go too
    /// (browsers build the page from the source);
    /// otherwise `angular-html-parser`'s table, against the direct parent only (Angular's compiler builds it).
    whatwg: bool,
    /// `{{ }}` and directive names.
    vue: bool,
    /// Root elements are SFC blocks: raw text unless `<template>` in HTML.
    sfc: bool,
    /// `{{ }}` (closed outside quotes) and control flow blocks.
    angular: bool,
    /// `/>` closes every element (Prettier's `canSelfClose`); otherwise only void and foreign ones, as in browsers.
    self_close_any: bool,
    /// `<//>` closes the innermost element (Prettier's `allowHtmComponentClosingTags`).
    htm_closing: bool,
}

impl Rules {
    const HTML: Self = Self {
        html: true,
        whatwg: true,
        vue: false,
        sfc: false,
        angular: false,
        self_close_any: false,
        htm_closing: true,
    };

    const VUE: Self = Self {
        html: false,
        whatwg: false,
        vue: true,
        sfc: true,
        angular: false,
        self_close_any: true,
        htm_closing: false,
    };

    /// A Vue file that turned out to be an HTML page.
    const VUE_HTML: Self = Self { vue: true, ..Self::HTML };

    /// Angular's compiler rejects `<div/>`, but no valid template depends on reading it otherwise.
    const ANGULAR: Self = Self {
        whatwg: false,
        angular: true,
        self_close_any: true,
        htm_closing: false,
        ..Self::HTML
    };
}

/// What the content at the bottom of the stack inherits;
/// a conditional comment's content starts where the comment is.
#[derive(Clone, Copy)]
struct Context {
    namespace: Namespace,
    v_pre: bool,
}

impl Context {
    const ROOT: Self = Self { namespace: Namespace::Html, v_pre: false };
}

enum Stop {
    Error(Diagnostic),
    /// A Vue file turned out to be an HTML page; parse again with HTML rules.
    Html,
}

type PResult<T> = Result<T, Stop>;

fn error<T>(kind: DiagnosticKind, span: Span) -> PResult<T> {
    Err(Stop::Error(Diagnostic::new(kind, span)))
}

/// Where a tag name ends, as in the HTML tokenizer: quotes, `<` and `=` are part of it.
fn is_tag_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'>' | b'/')
}

/// Where an attribute name ends, past its first character: as a tag name, at `=`, or at a quote (an error then).
fn is_attribute_name_end(b: u8) -> bool {
    is_tag_name_end(b) || matches!(b, b'=' | b'"' | b'\'')
}

/// Where the leading ASCII whitespace of `bytes` ends and the trailing one starts.
/// For whitespace-only input they cross (`(len, 0)`): the content is `lead..lead.max(trail)`.
fn trim(bytes: &[u8]) -> (usize, usize) {
    (bytes.len() - bytes.trim_ascii_start().len(), bytes.trim_ascii_end().len())
}

/// An element (or an Angular block) whose end has not been seen yet.
struct Open<'a> {
    id: NodeId,
    /// The tag name as written, for matching end tags; empty for a block.
    name: &'a str,
    block: bool,
    tag: Tag,
    /// What the element's children inherit.
    context: Context,
}

struct Builder<'a> {
    allocator: &'a Allocator,
    source: &'a str,
    /// The source up to where the current parse stops.
    bytes: &'a [u8],
    rules: Rules,
    /// What the bottom of the stack inherits, and where its nodes go.
    base: Context,
    base_parent: NodeId,
    pos: usize,
    stack: Vec<Open<'a>>,
    nodes: ArenaVec<'a, Node<'a>>,
}

impl<'a> Builder<'a> {
    /// `nodes` holds the document node.
    fn new(
        allocator: &'a Allocator,
        source: &'a str,
        rules: Rules,
        nodes: ArenaVec<'a, Node<'a>>,
    ) -> Self {
        Self {
            allocator,
            source,
            bytes: source.as_bytes(),
            rules,
            base: Context::ROOT,
            base_parent: NodeId::DOCUMENT,
            pos: 0,
            stack: vec![],
            nodes,
        }
    }

    fn run(&mut self) -> PResult<()> {
        while self.pos < self.bytes.len() {
            if self.at_markup(self.pos) {
                self.markup()?;
            } else {
                self.text(self.bytes.len(), true)?;
            }
        }
        while let Some(open) = self.stack.pop() {
            if open.block {
                let start = self.nodes[open.id.index()].span.start as usize;
                return error(DiagnosticKind::UnclosedBlock, span(start, start + 1));
            }
            self.close(&open, None, self.bytes.len());
        }
        Ok(())
    }

    /// Parses `[start, end)` as the children of `parent`, from scratch, with what the current position inherits.
    /// Returns whether it parsed; if not, nothing was added.
    fn parse_within(&mut self, parent: NodeId, start: usize, end: usize) -> bool {
        let len = self.nodes.len();
        let mut sub = Builder {
            allocator: self.allocator,
            source: self.source,
            bytes: &self.source.as_bytes()[..end],
            rules: self.rules,
            base: self.context(),
            base_parent: parent,
            pos: start,
            stack: vec![],
            nodes: std::mem::replace(&mut self.nodes, ArenaVec::new_in(&self.allocator)),
        };
        let parsed = sub.run().is_ok();
        self.nodes = sub.nodes;
        if !parsed {
            self.nodes.truncate(len);
            let parent = &mut self.nodes[parent.index()];
            parent.first_child = None;
            parent.last_child = None;
        }
        parsed
    }

    // --- Scanning helpers

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    /// The first offset from `at` whose byte fails `pred`.
    fn skip(&self, at: usize, mut pred: impl FnMut(u8) -> bool) -> usize {
        self.bytes[at..].iter().position(|&b| !pred(b)).map_or(self.bytes.len(), |i| at + i)
    }

    fn skip_ws(&self, at: usize) -> usize {
        self.skip(at, |b| b.is_ascii_whitespace())
    }

    fn tag_name_end(&self, at: usize) -> usize {
        self.skip(at, |b| !is_tag_name_end(b))
    }

    /// The offset of `pat` in `[from, limit)`.
    fn find(&self, from: usize, limit: usize, pat: &[u8]) -> Option<usize> {
        let haystack = &self.bytes[from..limit];
        match pat {
            [b] => memchr(*b, haystack),
            _ => memmem::find(haystack, pat),
        }
        .map(|i| from + i)
    }

    fn starts_with_ci(&self, at: usize, pat: &str) -> bool {
        self.bytes[at..].get(..pat.len()).is_some_and(|b| b.eq_ignore_ascii_case(pat.as_bytes()))
    }

    /// `<` followed by what can start markup; any other `<` is text.
    fn at_markup(&self, at: usize) -> bool {
        self.bytes[at] == b'<'
            && self
                .byte(at + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?'))
    }

    /// What the current element's next child inherits.
    fn context(&self) -> Context {
        self.stack.last().map_or(self.base, |open| open.context)
    }

    /// Whether `{{ }}` is an interpolation here.
    fn interpolates(&self) -> bool {
        (self.rules.vue || self.rules.angular) && !self.context().v_pre
    }

    fn at_sfc_root(&self) -> bool {
        self.rules.sfc && self.stack.is_empty() && self.base_parent == NodeId::DOCUMENT
    }

    /// The tag table as the dialect reads it: the one place for case and namespace policy.
    fn tag(&self, name: &str, lowercase: &str, namespace: Namespace) -> Tag {
        let table = tags::html(lowercase, self.rules.whatwg);
        if self.rules.html {
            if namespace == Namespace::Html {
                return table;
            }
            // `angular-html-parser` reads raw text by name in every namespace, and nothing else of the table.
            // SVG `<title>` is the exception it declares but its tokenizer misses (DIVERGENCES.md#svg-title-holds-markup).
            let content = if lowercase == "title" { Content::Normal } else { table.content };
            return Tag { content, ..Tag::default() };
        }
        // Vue: void tags and `pre` by exact name; raw text by any case, in HTML only; no implicit closing.
        let exact = tags::html(name, false);
        let html = namespace == Namespace::Html;
        Tag {
            void: exact.void,
            content: if html { table.content } else { Content::Normal },
            ignores_first_lf: html && exact.ignores_first_lf,
            ..Tag::default()
        }
    }

    // --- Tree

    /// Appends a node as the last child of the current element.
    fn push(&mut self, kind: NodeKind<'a>, span: Span) -> NodeId {
        let parent = self.stack.last().map_or(self.base_parent, |open| open.id);
        let id = NodeId::from_index(self.nodes.len());
        let prev = self.nodes[parent.index()].last_child;
        self.nodes.push(Node {
            kind,
            span,
            parent: Some(parent),
            next: None,
            first_child: None,
            last_child: None,
        });
        match prev {
            Some(prev) => self.nodes[prev.index()].next = Some(id),
            None => self.nodes[parent.index()].first_child = Some(id),
        }
        self.nodes[parent.index()].last_child = Some(id);
        id
    }

    fn element_mut(&mut self, id: NodeId) -> &mut Element<'a> {
        match &mut self.nodes[id.index()].kind {
            NodeKind::Element(el) => el,
            _ => unreachable!("only elements are opened"),
        }
    }

    /// `at` is where the element's content ends when it has no end tag.
    fn close(&mut self, open: &Open<'a>, end_tag: Option<EndTag>, at: usize) {
        self.element_mut(open.id).end_tag = end_tag;
        let node = &mut self.nodes[open.id.index()];
        let end = end_tag.map_or(at, |tag| tag.span.end as usize);
        node.span = span(node.span.start as usize, end);
    }

    /// Whether `open` may go without an end tag when its parent's closes it
    /// (closed by its parent, or a `/>` HTML ignored: `<my-el />`, closed as in browsers).
    fn omittable(&self, open: &Open<'a>) -> bool {
        open.tag.closed_by_parent
            || matches!(&self.nodes[open.id.index()].kind, NodeKind::Element(el) if el.self_closing)
    }

    /// Closes the open elements above `stack[index]` at `at`, when each is omittable;
    /// `false`, closing none, when one is not.
    fn close_omitted_above(&mut self, index: usize, at: usize) -> bool {
        let omittable = self.stack[index + 1..].iter().all(|open| self.omittable(open));
        if omittable {
            self.close_down_to(index + 1, at);
        }
        omittable
    }

    /// Closes the open elements from `stack[len]` up at `at`, without end tags.
    fn close_down_to(&mut self, len: usize, at: usize) {
        while self.stack.len() > len {
            let open = self.stack.pop().expect("len < stack.len()");
            self.close(&open, None, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors() {
        use DiagnosticKind::*;
        for (source, dialect, kind, at) in [
            ("<!-- a", Dialect::Html, UnterminatedComment, 0),
            ("<!x", Dialect::Html, UnterminatedComment, 0),
            ("<![CDATA[ a", Dialect::Html, UnterminatedCdata, 0),
            ("<!doctype html", Dialect::Html, UnterminatedDocType, 0),
            ("<div", Dialect::Html, UnterminatedStartTag, 0),
            ("</div", Dialect::Html, UnterminatedEndTag, 0),
            ("<p>{{ a</p>", Dialect::Angular, UnterminatedInterpolation, 3),
            ("{{ '<b>' }}", Dialect::Angular, UnterminatedInterpolation, 0),
            ("<a b='c>", Dialect::Html, UnterminatedAttributeValue, 5),
            ("<a / b>", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<a \"b\">", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<a b\"c=1>", Dialect::Html, UnexpectedCharacterInTag, 4),
            ("<a b=\"c(\"d\")\">", Dialect::Html, UnexpectedCharacterInTag, 10),
            ("<script>a", Dialect::Html, UnterminatedRawText, 0),
            ("<script><!--<script></script>", Dialect::Html, UnterminatedRawText, 0),
            ("</div>", Dialect::Html, UnexpectedEndTag, 0),
            ("<div><span></div>", Dialect::Html, UnexpectedEndTag, 11),
            ("<br></br>", Dialect::Html, VoidEndTag, 4),
            ("<template>{{ a</template>", Dialect::Vue, UnterminatedInterpolation, 10),
            ("<template><p><div></p></template>", Dialect::Vue, UnexpectedEndTag, 18),
            ("<docs>a", Dialect::Vue, UnterminatedRawText, 0),
            ("a<//>", Dialect::Html, UnexpectedEndTag, 1),
        ] {
            let allocator = Allocator::default();
            let diagnostic = Parser::new(&allocator, source, dialect).parse().unwrap_err();
            assert_eq!((diagnostic.kind, diagnostic.span.start), (kind, at), "{source:?}");
        }
    }

    /// A conditional comment whose content does not parse keeps it unparsed instead of failing.
    #[test]
    fn unparsable_conditional_comment() {
        let allocator = Allocator::default();
        let root =
            Parser::new(&allocator, "<!--[if IE]></p><![endif]-->", Dialect::Html).parse().unwrap();
        let (id, node) = root.children(NodeId::DOCUMENT).next().unwrap();
        let NodeKind::ConditionalComment(comment) = &node.kind else { panic!() };
        assert!(!comment.parsed);
        assert!(root.children(id).next().is_none());
    }
}
