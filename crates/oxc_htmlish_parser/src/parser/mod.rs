//! Parser entry point.
//!
//! One pass over the bytes builds the tree directly; there is no token stream.
//! Tree construction is limited to the tag table (`tags.rs`):
//! void elements, raw text, implicit closing, what `head` holds.

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
    /// TODO: ICU expressions and start-tag comments are not parsed yet (an error), nor `@let` (plain text).
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
    /// otherwise Angular's table, against the direct parent only (Angular's compiler builds it).
    whatwg: bool,
    /// `{{ }}` and directive names.
    vue: bool,
    /// A Vue SFC, read by Vue's SFC tokenizer: root elements are SFC blocks (raw text unless `<template>` in HTML);
    /// a void element's end tag is text, a quote may open an attribute name or sit in a directive name, `</` ends a start tag,
    /// and RCDATA reads `{{ }}` as that tokenizer does.
    sfc: bool,
    /// `{{ }}` (closed outside quotes) and control flow blocks, and Angular's lexer in tags:
    /// its whitespace (VT, U+000E–U+001F, NBSP), namespace prefixes, `<!-->` read on to `-->`,
    /// whitespace around a raw text element's end tag name and no script data escapes,
    /// `=` ending an unquoted value and opening a nameless attribute.
    angular: bool,
    /// Whitespace may come before an end tag's name (`</ div>`), as Vue's and Angular's compilers read it;
    /// otherwise `</` and whitespace open a bogus comment, as in the HTML tokenizer.
    end_tag_whitespace: bool,
    /// A comment closes at `--!>` too, as in the HTML tokenizer; otherwise at `-->` only, as Vue's and Angular's compilers read it.
    bang_comment_end: bool,
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
        end_tag_whitespace: false,
        bang_comment_end: true,
    };

    const VUE: Self = Self {
        html: false,
        whatwg: false,
        vue: true,
        sfc: true,
        angular: false,
        self_close_any: true,
        htm_closing: false,
        end_tag_whitespace: true,
        bang_comment_end: false,
    };

    /// A Vue file that turned out to be an HTML page.
    const VUE_HTML: Self = Self { vue: true, ..Self::HTML };

    /// Angular's compiler rejects `<div/>`, but no valid template depends on reading it otherwise.
    const ANGULAR: Self = Self {
        whatwg: false,
        angular: true,
        self_close_any: true,
        htm_closing: false,
        end_tag_whitespace: true,
        bang_comment_end: false,
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

/// Where an attribute name ends, past its first character and whitespace: as a tag name, at `=`, or at a quote (an error then).
fn ends_attribute_name(b: u8) -> bool {
    matches!(b, b'>' | b'/' | b'=' | b'"' | b'\'')
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
            // The text stops at markup, or at the end
            if !self.at_markup(self.pos) {
                self.text(self.bytes.len(), true)?;
                if self.pos == self.bytes.len() {
                    break;
                }
            }
            self.markup()?;
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
    // The hot ones inline, with their dialect-only paths split out and the dialect checks of `at_markup` kept out of line:
    // grown by those, they stop inlining

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    /// The first offset from `at` whose byte fails `pred`.
    fn skip(&self, at: usize, mut pred: impl FnMut(u8) -> bool) -> usize {
        self.bytes[at..].iter().position(|&b| !pred(b)).map_or(self.bytes.len(), |i| at + i)
    }

    /// The length of the whitespace at `at` in a tag or a block's head, 0 for none: ASCII whitespace, as in the HTML tokenizer;
    /// in Angular also VT, U+000E–U+001F and NBSP, as its compiler reads them (DIVERGENCES.md#whitespace-in-tags).
    fn ws_len(&self, at: usize) -> usize {
        match self.byte(at) {
            Some(b) if b.is_ascii_whitespace() => 1,
            Some(0x0B | 0x0E..=0x1F) if self.rules.angular => 1,
            Some(0xC2) if self.rules.angular && self.byte(at + 1) == Some(0xA0) => 2,
            _ => 0,
        }
    }

    #[inline]
    fn skip_ws(&self, at: usize) -> usize {
        if self.rules.angular {
            self.skip_ws_angular(at)
        } else {
            self.skip(at, |b| b.is_ascii_whitespace())
        }
    }

    fn skip_ws_angular(&self, mut at: usize) -> usize {
        while let n @ 1.. = self.ws_len(at) {
            at += n;
        }
        at
    }

    /// The first offset from `at` that is whitespace or a byte `end` takes.
    #[inline]
    fn until_ws_or(&self, at: usize, end: impl Fn(u8) -> bool) -> usize {
        if self.rules.angular {
            self.until_ws_or_angular(at, end)
        } else {
            self.skip(at, |b| !b.is_ascii_whitespace() && !end(b))
        }
    }

    fn until_ws_or_angular(&self, mut at: usize, end: impl Fn(u8) -> bool) -> usize {
        while self.byte(at).is_some_and(|b| !end(b)) && self.ws_len(at) == 0 {
            at += 1;
        }
        at
    }

    #[inline]
    fn tag_name_end(&self, at: usize) -> usize {
        self.until_ws_or(at, |b| matches!(b, b'>' | b'/'))
    }

    /// Where an end tag's name starts after `</` at `start`:
    /// right there, as in the HTML tokenizer; past whitespace, as Vue's and Angular's compilers read it (DIVERGENCES.md#end-tag-after-whitespace).
    fn end_tag_name_start(&self, start: usize) -> usize {
        if self.rules.end_tag_whitespace { self.skip_ws(start + 2) } else { start + 2 }
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

    /// `<` followed by a letter, `/` or `!`: a tag, a comment or a declaration starts (Angular's `_isTagStart`).
    fn at_tag_start(&self, at: usize) -> bool {
        self.bytes[at] == b'<'
            && self
                .byte(at + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!'))
    }

    /// `<` followed by what can start markup (`<?` too); any other `<` is text.
    /// In a Vue SFC, a void element's end tag (`</br>`) is text too: Vue drops it (DIVERGENCES.md#void-element-end-tags).
    #[inline]
    fn at_markup(&self, at: usize) -> bool {
        (self.at_tag_start(at) || self.bytes[at..].starts_with(b"<?"))
            && !(self.rules.sfc && self.byte(at + 1) == Some(b'/') && self.at_void_end_tag(at))
            && !(self.rules.angular && self.at_prefix_without_name(at))
    }

    /// Angular: `<svg:` with no local name after the prefix is no tag; its compiler reads the `<` as text.
    #[inline(never)]
    fn at_prefix_without_name(&self, at: usize) -> bool {
        let colon = self.skip(at + 1, |b| b.is_ascii_alphanumeric());
        colon > at + 1
            && self.byte(colon) == Some(b':')
            && self.tag_name_end(colon + 1) == colon + 1
    }

    /// The end tag at `at` (`</`) closes a void element.
    #[inline(never)]
    fn at_void_end_tag(&self, at: usize) -> bool {
        let name_start = self.end_tag_name_start(at);
        tags::is_void(&self.source[name_start..self.tag_name_end(name_start)])
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
    /// `svg_prefixed`: Angular's `svg:` prefix named the namespace.
    fn tag(&self, name: &str, lowercase: &str, namespace: Namespace, svg_prefixed: bool) -> Tag {
        let table = tags::html(lowercase, self.rules.whatwg);
        if self.rules.html {
            if namespace == Namespace::Html {
                return table;
            }
            // `angular-html-parser` reads raw text by name in every namespace, and nothing else of the table.
            // SVG `<title>` is the exception it declares but its tokenizer misses, as browsers read it (DIVERGENCES.md#svg-title);
            // Angular's compiler applies it to `svg:title` only.
            let markup = !self.rules.angular || svg_prefixed;
            let content =
                if lowercase == "title" && markup { Content::Normal } else { table.content };
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

    /// Closes the open elements above the first `len` at `at`, when each is omittable;
    /// `false`, closing none, when one is not.
    fn close_omitted_above(&mut self, len: usize, at: usize) -> bool {
        let omittable = self.stack[len..].iter().all(|open| self.omittable(open));
        if omittable {
            self.close_down_to(len, at);
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
            // Angular reads the `{{` as text, then the `}` closes no block
            ("{{ '<b>' }}", Dialect::Angular, UnexpectedBlockClose, 9),
            ("<a b='c>", Dialect::Html, UnterminatedAttributeValue, 5),
            ("<a / b>", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<a \"b\">", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<a b\"c=1>", Dialect::Html, UnexpectedCharacterInTag, 4),
            ("<a b=\"c(\"d\")\">", Dialect::Html, UnexpectedCharacterInTag, 10),
            ("<template><a b\"c=1></a></template>", Dialect::Vue, UnexpectedCharacterInTag, 14),
            ("<template><a / b></a></template>", Dialect::Vue, UnexpectedCharacterInTag, 13),
            ("<script>a", Dialect::Html, UnterminatedRawText, 0),
            ("<script><!--<script></script>", Dialect::Html, UnterminatedRawText, 0),
            ("</div>", Dialect::Html, UnexpectedEndTag, 0),
            ("<div><span></div>", Dialect::Html, UnexpectedEndTag, 11),
            ("<br></br>", Dialect::Html, VoidEndTag, 4),
            ("<template>{{ a</template>", Dialect::Vue, UnterminatedInterpolation, 10),
            ("<template><p><div></p></template>", Dialect::Vue, UnexpectedEndTag, 18),
            ("<docs>a", Dialect::Vue, UnterminatedRawText, 0),
            ("a<//>", Dialect::Html, UnexpectedEndTag, 1),
            // An omitted element's end tag where WHATWG reads it with an error
            ("<div></body>", Dialect::Html, UnexpectedEndTag, 5),
            ("<p>x</p></head>", Dialect::Html, UnexpectedEndTag, 8),
            ("<body></body></body>", Dialect::Html, UnexpectedEndTag, 13),
            ("<html></html></body>", Dialect::Html, UnexpectedEndTag, 13),
            ("</html></body>", Dialect::Html, UnexpectedEndTag, 7),
            ("</head></head>", Dialect::Html, UnexpectedEndTag, 7),
            ("<template></body></template>", Dialect::Vue, UnexpectedEndTag, 10),
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
        for source in ["<!--[if IE]></p><![endif]-->", "<!--[if IE]></body><![endif]-->"] {
            let root = Parser::new(&allocator, source, Dialect::Html).parse().unwrap();
            let (id, node) = root.children(NodeId::DOCUMENT).next().unwrap();
            let NodeKind::ConditionalComment(comment) = &node.kind else { panic!() };
            assert!(!comment.parsed, "{source:?}");
            assert!(root.children(id).next().is_none());
        }
    }
}
