//! Parser entry point.
//!
//! One pass over the bytes builds the tree directly; there is no token stream.
//! Tree construction is limited to the tag table (`tags.rs`):
//! void elements, raw text, implicit closing.

use memchr::{memchr, memchr2, memmem};
use oxc_allocator::{Allocator, ArenaBox, ArenaVec};

use crate::ast::*;
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::span::{Span, span};
use crate::tags::{self, Tag};
use crate::vue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Html,
    /// A Vue SFC.
    /// A file with a root `<html>` or `<!doctype html>` is read with HTML rules instead
    /// (`{{ }}` and directives are still Vue).
    Vue,
    /// An Angular template: HTML with `{{ }}`, control flow blocks (`@if (a) { … }`), and `/>` closing any element.
    /// ICU expressions and `@let` are not parsed yet (plain text).
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
            Dialect::Html => Rules { html: true, vue: false, sfc: false, angular: false },
            Dialect::Vue => Rules { html: false, vue: true, sfc: true, angular: false },
            Dialect::Angular => Rules { html: true, vue: false, sfc: false, angular: true },
        };
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
                    // Start over in the same buffer, keeping only the document node.
                    nodes = builder.nodes;
                    nodes.truncate(1);
                    nodes[0].first_child = None;
                    nodes[0].last_child = None;
                    rules = Rules { html: true, sfc: false, ..rules };
                }
            }
        }
    }
}

/// What a dialect switches on.
#[derive(Clone, Copy)]
struct Rules {
    /// `angular-html-parser`'s reading of tags rather than Vue's:
    /// case-insensitive tag table limited to the HTML namespace, implicit closing,
    /// `/>` closing only void and foreign elements, `</ name>` as an end tag.
    html: bool,
    /// `{{ }}` and directive names.
    vue: bool,
    /// Root elements are SFC blocks: raw text unless `<template>` in HTML.
    sfc: bool,
    /// `{{ }}` (closed outside quotes) and control flow blocks.
    angular: bool,
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

fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'>' | b'<' | b'/' | b'\'' | b'"' | b'=')
}

/// Where the leading ASCII whitespace of `bytes` ends and the trailing one starts.
/// For whitespace-only input they cross (`(len, 0)`): the content is `lead..lead.max(trail)`.
fn trim(bytes: &[u8]) -> (usize, usize) {
    (bytes.len() - bytes.trim_ascii_start().len(), bytes.trim_ascii_end().len())
}

/// The block names `angular-html-parser` starts a block at, by prefix (`@iffy` is a block named `iffy`).
const BLOCK_NAMES: [&[u8]; 13] = [
    b"if",
    b"else",
    b"for",
    b"switch",
    b"case",
    b"default",
    b"empty",
    b"defer",
    b"placeholder",
    b"loading",
    b"boundary",
    b"error",
    b"content",
];

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
            rules: Rules { sfc: false, ..self.rules },
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

    // ---
    // Scanning helpers

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    /// The first offset from `at` whose byte fails `pred`.
    fn skip(&self, at: usize, pred: impl Fn(u8) -> bool) -> usize {
        self.bytes[at..].iter().position(|&b| !pred(b)).map_or(self.bytes.len(), |i| at + i)
    }

    fn skip_ws(&self, at: usize) -> usize {
        self.skip(at, |b| b.is_ascii_whitespace())
    }

    fn name_end(&self, at: usize) -> usize {
        self.skip(at, |b| !is_name_end(b))
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

    /// An Angular block starts at `at`.
    fn at_block(&self, at: usize) -> bool {
        self.byte(at) == Some(b'@')
            && BLOCK_NAMES.iter().any(|name| self.bytes[at + 1..].starts_with(name))
    }

    /// The offsets in `[from, limit)` outside string literals, as Angular expressions read them:
    /// `'`, `"` and `` ` `` open one, and a `\` skips the next byte.
    fn unquoted(&self, from: usize, limit: usize) -> impl Iterator<Item = usize> + '_ {
        let mut quote = None;
        let mut at = from;
        std::iter::from_fn(move || {
            while at < limit {
                let i = at;
                at += 1;
                match (self.bytes[i], quote) {
                    (b'\\', _) => at += 1,
                    (b, Some(q)) if b == q => quote = None,
                    (_, Some(_)) => {}
                    (q @ (b'\'' | b'"' | b'`'), None) => quote = Some(q),
                    _ => return Some(i),
                }
            }
            None
        })
    }

    fn at_sfc_root(&self) -> bool {
        self.rules.sfc && self.stack.is_empty()
    }

    /// The tag table as the dialect reads it: the one place for case and namespace policy.
    fn tag(&self, name: &str, lowercase: &str, namespace: Namespace) -> Tag {
        let table = tags::html(lowercase);
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
        let exact = tags::html(name);
        let html = namespace == Namespace::Html;
        Tag {
            void: exact.void,
            content: if html { table.content } else { Content::Normal },
            ignores_first_lf: html && exact.ignores_first_lf,
            ..Tag::default()
        }
    }

    // ---
    // Tree

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

    // ---
    // Text

    /// Text from `pos` up to `limit`, or to the next markup when `markup_ends`.
    /// `{{ }}` and Angular blocks split it.
    fn text(&mut self, limit: usize, markup_ends: bool) -> PResult<()> {
        let interpolate = self.interpolates();
        let blocks = self.rules.angular && markup_ends;
        let mut start = self.pos;
        while self.pos < limit {
            let haystack = &self.bytes[self.pos..limit];
            let found = if blocks {
                haystack.iter().position(|b| matches!(b, b'<' | b'{' | b'@' | b'}'))
            } else {
                match (markup_ends, interpolate) {
                    (true, true) => memchr2(b'<', b'{', haystack),
                    (true, false) => memchr(b'<', haystack),
                    (false, true) => memchr(b'{', haystack),
                    (false, false) => None,
                }
            };
            let Some(i) = found else {
                self.pos = limit;
                break;
            };
            self.pos += i;
            match self.bytes[self.pos] {
                b'<' => {
                    if self.at_markup(self.pos) {
                        break;
                    }
                    self.pos += 1;
                    continue;
                }
                b'@' if blocks && self.at_block(self.pos) => {
                    self.push_text(start, self.pos);
                    self.block_start()?;
                    start = self.pos;
                    continue;
                }
                b'}' if blocks => {
                    self.push_text(start, self.pos);
                    self.block_close()?;
                    start = self.pos;
                    continue;
                }
                b'@' | b'}' => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            if self.byte(self.pos + 1) != Some(b'{') {
                self.pos += 1;
                continue;
            }
            let open = self.pos;
            let close = if self.rules.angular {
                self.interpolation_end(open + 2, limit)
            } else {
                // Vue closes at the first `}}`, quotes and tags included.
                self.find(open + 2, limit, b"}}")
            };
            let Some(close) = close else {
                return error(DiagnosticKind::UnterminatedInterpolation, span(open, open + 2));
            };
            self.push_text(start, open);
            self.push(
                NodeKind::Interpolation(Interpolation { expression: span(open + 2, close) }),
                span(open, close + 2),
            );
            self.pos = close + 2;
            start = self.pos;
        }
        self.push_text(start, self.pos);
        Ok(())
    }

    /// Angular: the `}}` closing an interpolation, outside string literals;
    /// after a `//` comment, quotes no longer count.
    fn interpolation_end(&self, from: usize, limit: usize) -> Option<usize> {
        for at in self.unquoted(from, limit) {
            match &self.bytes[at..limit] {
                [b'}', b'}', ..] => return Some(at),
                [b'/', b'/', ..] => return self.find(at + 2, limit, b"}}"),
                _ => {}
            }
        }
        None
    }

    // ---
    // Angular blocks

    /// `@name (parameters) {`, or a block without a body; `pos` is at `@`.
    fn block_start(&mut self) -> PResult<()> {
        let start = self.pos;
        // The name may hold whitespace after its first character (`else if`).
        let name_start = start + 1;
        let mut at = self.skip(name_start, |b| {
            b.is_ascii_alphanumeric() || b == b'_' || b.is_ascii_whitespace()
        });
        let name_end = name_start + trim(&self.bytes[name_start..at]).1;
        let mut parameters = ArenaVec::new_in(&self.allocator);
        let mut open_end = name_end;
        if self.byte(at) == Some(b'(') {
            at = self.block_parameters(at + 1, &mut parameters)?;
            open_end = at;
            at = self.skip_ws(at);
        }
        let name = &self.source[name_start..name_end];
        let body = self.byte(at) == Some(b'{');
        // `@default never;` has no body; cases fall through to the next one without one.
        let never = name.split_ascii_whitespace().eq(["default", "never"])
            && !name.contains('\n')
            && self.byte(at) == Some(b';');
        if body || never {
            open_end = at + 1;
        } else if !(matches!(name, "case" | "default") && self.at_block(at)) {
            return error(DiagnosticKind::IncompleteBlock, span(start, open_end));
        }
        let block = Block {
            name: span(name_start, name_end),
            parameters,
            open: span(start, open_end),
            close: None,
        };
        let id = self
            .push(NodeKind::Block(ArenaBox::new_in(block, &self.allocator)), span(start, open_end));
        self.pos = open_end;
        if body {
            let context = self.context();
            self.stack.push(Open { id, name: "", block: true, tag: Tag::default(), context });
        }
        Ok(())
    }

    /// The parameters after `(`, up to the `)`, which the returned offset follows.
    fn block_parameters(&self, from: usize, out: &mut ArenaVec<'a, Span>) -> PResult<usize> {
        let push = |start: usize, end: usize, out: &mut ArenaVec<'a, Span>| {
            let (lead, trail) = trim(&self.bytes[start..end]);
            if lead < trail {
                out.push(span(start + lead, start + trail));
            }
        };
        let (mut param_start, mut depth) = (from, 0);
        for at in self.unquoted(from, self.bytes.len()) {
            match self.bytes[at] {
                b'(' => depth += 1,
                b')' if depth > 0 => depth -= 1,
                b')' => {
                    push(param_start, at, out);
                    return Ok(at + 1);
                }
                b';' => {
                    push(param_start, at, out);
                    param_start = at + 1;
                }
                _ => {}
            }
        }
        error(DiagnosticKind::IncompleteBlock, span(from - 1, self.bytes.len()))
    }

    /// `}`: closes the innermost block, and the elements in it whose end tag may be omitted.
    fn block_close(&mut self) -> PResult<()> {
        let at = self.pos;
        let index = self.stack.iter().rposition(|open| open.block);
        if !index.is_some_and(|index| self.close_omitted_above(index, at)) {
            return error(DiagnosticKind::UnexpectedBlockClose, span(at, at + 1));
        }
        let open = self.stack.pop().expect("the block is on the stack");
        let node = &mut self.nodes[open.id.index()];
        node.span = span(node.span.start as usize, at + 1);
        if let NodeKind::Block(block) = &mut node.kind {
            block.close = Some(span(at, at + 1));
        }
        self.pos = at + 1;
        Ok(())
    }

    /// Closes the open elements above `stack[index]` at `at`, when each may go without an end tag
    /// (closed by its parent, or a `/>` HTML ignored: `<my-el />`, closed as in browsers);
    /// `false`, closing none, when one may not.
    fn close_omitted_above(&mut self, index: usize, at: usize) -> bool {
        let omittable = self.stack[index + 1..].iter().all(|open| {
            open.tag.closed_by_parent
                || matches!(&self.nodes[open.id.index()].kind, NodeKind::Element(el) if el.self_closing)
        });
        if omittable {
            while self.stack.len() > index + 1 {
                let open = self.stack.pop().expect("index < len");
                self.close(&open, None, at);
            }
        }
        omittable
    }

    fn push_text(&mut self, mut start: usize, end: usize) {
        if let Some(open) = self.stack.last()
            && open.tag.ignores_first_lf
            && self.nodes[open.id.index()].first_child.is_none()
        {
            let id = open.id;
            let lf = match &self.bytes[start..end] {
                [b'\r', b'\n', ..] => 2,
                [b'\n' | b'\r', ..] => 1,
                _ => 0,
            };
            let element = self.element_mut(id);
            if lf > 0 && !element.leading_lf {
                start += lf;
                element.leading_lf = true;
            }
        }
        if start == end {
            return;
        }

        let bytes = &self.bytes[start..end];
        let kind = |ws: &[u8]| {
            if ws.is_empty() {
                Whitespace::None
            } else if ws.iter().any(|b| matches!(b, b'\n' | b'\r')) {
                Whitespace::Newline
            } else {
                Whitespace::Space
            }
        };
        let (lead, trail) = trim(bytes);
        let text = Text {
            content: span(start + lead, start + lead.max(trail)),
            leading: kind(&bytes[..lead]),
            trailing: kind(&bytes[trail..]),
        };
        self.push(NodeKind::Text(text), span(start, end));
    }

    // ---
    // Markup

    fn markup(&mut self) -> PResult<()> {
        let start = self.pos;
        match self.bytes[start + 1] {
            // Vue (as browsers) reads `</` without a name as a bogus comment;
            // `angular-html-parser` skips whitespace before the name (`</ div >`).
            b'/' if !self.rules.html
                && self.byte(start + 2).is_some_and(|b| !b.is_ascii_alphabetic() && b != b'>') =>
            {
                self.bogus_comment(2)
            }
            b'/' => self.end_tag(),
            b'!' if self.starts_with_ci(start, "<!--") => self.comment(),
            b'!' if self.starts_with_ci(start, "<![CDATA[") => {
                let Some(close) = self.find(start + 9, self.bytes.len(), b"]]>") else {
                    return error(DiagnosticKind::UnterminatedCdata, span(start, start + 9));
                };
                self.pos = close + 3;
                self.push(NodeKind::Cdata, span(start, self.pos));
                Ok(())
            }
            b'!' if self.starts_with_ci(start, "<!doctype") => self.doctype(),
            b'!' => self.bogus_comment(2),
            b'?' => self.bogus_comment(1),
            _ => self.start_tag(),
        }
    }

    fn comment(&mut self) -> PResult<()> {
        let start = self.pos;
        let content_start = start + 4;
        let Some(close) = self.find(content_start, self.bytes.len(), b"-->") else {
            return error(DiagnosticKind::UnterminatedComment, span(start, content_start));
        };
        self.pos = close + 3;
        let span = span(start, self.pos);
        let content = self::span(content_start, close);
        if !self.conditional_comment(span, content) {
            self.push(NodeKind::Comment(Comment { content, bogus: false }), span);
        }
        Ok(())
    }

    /// The three IE conditional comment shapes Prettier recognizes; `false` for any other comment.
    fn conditional_comment(&mut self, span: Span, content: Span) -> bool {
        let text = content.slice(self.source);
        let base = content.start as usize;

        let Some(rest) = text.strip_prefix("[if") else {
            let is_end =
                text.strip_prefix("<!").is_some_and(|endif| endif.trim_ascii_start() == "[endif]");
            if is_end {
                self.push(NodeKind::ConditionalEnd, span);
            }
            return is_end;
        };
        let Some(close) = rest.find(']') else { return false };
        let condition = self::span(base + 3, base + 3 + close);
        let Some(after) = rest[close + 1..].strip_prefix('>') else { return false };
        if after == "<!" {
            self.push(NodeKind::ConditionalStart(ConditionalStart { condition }), span);
            return true;
        }
        let Some(data) = after
            .strip_suffix("[endif]")
            .map(str::trim_ascii_end)
            .and_then(|d| d.strip_suffix("<!"))
        else {
            return false;
        };
        let data_start = base + 3 + close + 2;
        let data_end = data_start + data.len();
        let content = self::span(data_start, data_end);
        let comment = ArenaBox::new_in(
            ConditionalComment { condition, content, parsed: true },
            &self.allocator,
        );
        let id = self.push(NodeKind::ConditionalComment(comment), span);
        if !self.parse_within(id, data_start, data_end)
            && let NodeKind::ConditionalComment(comment) = &mut self.nodes[id.index()].kind
        {
            comment.parsed = false;
        }
        true
    }

    /// `<!x>` / `<?x>`: data up to the next `>`.
    fn bogus_comment(&mut self, prefix: usize) -> PResult<()> {
        let start = self.pos;
        let Some(close) = self.find(start + prefix, self.bytes.len(), b">") else {
            return error(DiagnosticKind::UnterminatedComment, span(start, start + prefix));
        };
        self.pos = close + 1;
        let content = span(start + prefix, close);
        self.push(NodeKind::Comment(Comment { content, bogus: true }), span(start, self.pos));
        Ok(())
    }

    fn doctype(&mut self) -> PResult<()> {
        let start = self.pos;
        let value_from = start + 9;
        let Some(close) = self.find(value_from, self.bytes.len(), b">") else {
            return error(DiagnosticKind::UnterminatedDocType, span(start, value_from));
        };
        self.pos = close + 1;
        let (lead, trail) = trim(&self.bytes[value_from..close]);
        let value = span(value_from + lead, value_from + lead.max(trail));
        if self.at_sfc_root() && value.slice(self.source) == "html" {
            return Err(Stop::Html);
        }
        self.push(NodeKind::DocType(DocType { value }), span(start, self.pos));
        Ok(())
    }

    fn end_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        let name_start = self.skip_ws(start + 2);
        let name_end = self.name_end(name_start);
        let gt = self.skip_ws(name_end);
        if self.byte(gt) != Some(b'>') {
            return error(DiagnosticKind::UnterminatedEndTag, span(start, gt));
        }
        self.pos = gt + 1;
        let tag = EndTag { span: span(start, self.pos), name: span(name_start, name_end) };
        let name = &self.source[name_start..name_end];

        let mut buf = [0; 16];
        if self.tag(name, tags::lowercase(name, &mut buf), self.context().namespace).void {
            return error(DiagnosticKind::VoidEndTag, tag.span);
        }
        // Every dialect so far matches end tags case-insensitively.
        let Some(index) =
            self.stack.iter().rposition(|open| !open.block && open.name.eq_ignore_ascii_case(name))
        else {
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        };
        if !self.close_omitted_above(index, start) {
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        }
        let open = self.stack.pop().expect("index < len");
        self.close(&open, Some(tag), start);
        Ok(())
    }

    fn start_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        let name_end = self.name_end(start + 1);
        let name = span(start + 1, name_end);
        let name_str = name.slice(self.source);
        if self.at_sfc_root() && name_str.eq_ignore_ascii_case("html") {
            return Err(Stop::Html);
        }

        let mut attributes = ArenaVec::new_in(&self.allocator);
        let mut at = self.skip_ws(name_end);
        let self_closing = loop {
            match self.byte(at) {
                None | Some(b'<') => {
                    return error(DiagnosticKind::UnterminatedStartTag, span(start, at));
                }
                Some(b'>') => {
                    at += 1;
                    break false;
                }
                Some(b'/') if self.byte(at + 1) == Some(b'>') => {
                    at += 2;
                    break true;
                }
                Some(b'/' | b'"' | b'\'') => {
                    return error(DiagnosticKind::UnexpectedCharacterInTag, span(at, at + 1));
                }
                Some(_) => {
                    let attribute = self.attribute(at)?;
                    at = self.skip_ws(attribute.span.end as usize);
                    attributes.push(attribute);
                }
            }
        };
        self.pos = at;

        let mut buf = [0; 16];
        let lowercase = tags::lowercase(name_str, &mut buf);
        let key = if !self.rules.html || !name_str.bytes().any(|b| b.is_ascii_uppercase()) {
            name_str
        } else if lowercase.is_empty() {
            // Longer than every table entry: lowercased for the formatter's own tables.
            let mut key = name_str.to_owned();
            key.make_ascii_lowercase();
            self.allocator.alloc_str(&key)
        } else {
            self.allocator.alloc_str(lowercase)
        };

        if let Some(open) = self.stack.pop_if(|parent| parent.tag.closed_by.contains(&key)) {
            self.close(&open, None, start);
        }

        let inherited = self.context();
        let namespace = tags::namespace(key, inherited.namespace);
        let tag = self.tag(name_str, lowercase, namespace);
        // Angular: `/>` closes every element (Prettier's `canSelfClose`; Angular's compiler would reject `<div/>`).
        let closes_now = tag.void
            || (self_closing
                && (!self.rules.html || self.rules.angular || namespace != Namespace::Html));

        let v_pre = self.rules.vue
            && (inherited.v_pre || attributes.iter().any(|a| a.name.slice(self.source) == "v-pre"));
        if self.rules.vue && !v_pre {
            for attribute in &mut attributes {
                if let Some(directive) = vue::directive(self.allocator, self.source, attribute.name)
                {
                    attribute.kind =
                        AttributeKind::Vue(ArenaBox::new_in(directive, &self.allocator));
                }
            }
        }

        let content = if closes_now {
            Content::Normal
        } else if self.at_sfc_root() {
            if name_str == "template" && !has_non_html_lang(self.source, &attributes) {
                Content::Normal
            } else {
                Content::Raw
            }
        } else {
            tag.content
        };

        let tag_span = span(start, at);
        let element = Element {
            name,
            key,
            namespace,
            attributes,
            start_tag: tag_span,
            self_closing,
            void: tag.void,
            content,
            ignores_first_lf: tag.ignores_first_lf,
            leading_lf: false,
            end_tag: None,
        };
        let id = self.push(NodeKind::Element(ArenaBox::new_in(element, &self.allocator)), tag_span);
        let open = Open {
            id,
            name: name_str,
            block: false,
            tag,
            context: Context { namespace: tags::children_namespace(name_str, namespace), v_pre },
        };
        if closes_now {
            self.close(&open, None, at);
            return Ok(());
        }
        self.stack.push(open);
        match content {
            Content::Normal => Ok(()),
            Content::Raw => self.raw_text(tag_span, name_str, false),
            Content::Escapable => self.raw_text(tag_span, name_str, true),
        }
    }

    fn attribute(&self, start: usize) -> PResult<Attribute<'a>> {
        // The first character is always part of the name, `=` included (as in browsers).
        let name_end = self.name_end(start + 1);
        let name = span(start, name_end);
        let eq = self.skip_ws(name_end);
        if self.byte(eq) != Some(b'=') {
            return Ok(Attribute {
                name,
                value: None,
                kind: AttributeKind::Plain,
                span: span(start, name_end),
            });
        }

        let value_start = self.skip_ws(eq + 1);
        let (value, end) = if let Some(q @ (b'"' | b'\'')) = self.byte(value_start) {
            let Some(close) = self.find(value_start + 1, self.bytes.len(), &[q]) else {
                return error(
                    DiagnosticKind::UnterminatedAttributeValue,
                    span(value_start, self.bytes.len()),
                );
            };
            let quote = if q == b'"' { Quote::Double } else { Quote::Single };
            (AttributeValue { span: span(value_start + 1, close), quote }, close + 1)
        } else {
            // Browsers (and Vue) end an unquoted value at whitespace or `>` only:
            // `href=/a/b` and `src=a.png/` keep their slashes.
            let value_end = self.skip(value_start, |b| !b.is_ascii_whitespace() && b != b'>');
            (AttributeValue { span: span(value_start, value_end), quote: Quote::None }, value_end)
        };
        Ok(Attribute {
            name,
            value: Some(value),
            kind: AttributeKind::Plain,
            span: span(start, end),
        })
    }

    /// Content up to `</name>` (case-insensitive) as one text, `{{ }}` split when `escapable`.
    fn raw_text(&mut self, start_tag: Span, name: &str, escapable: bool) -> PResult<()> {
        let content_start = self.pos;
        let finder = memmem::Finder::new(b"</");
        let mut from = content_start;
        let (close_start, name_start, close_end) = loop {
            let Some(lt) = finder.find(&self.bytes[from..]).map(|i| from + i) else {
                return error(DiagnosticKind::UnterminatedRawText, start_tag);
            };
            let name_start = self.skip_ws(lt + 2);
            if self.starts_with_ci(name_start, name) {
                let gt = self.skip_ws(name_start + name.len());
                if self.byte(gt) == Some(b'>') {
                    break (lt, name_start, gt + 1);
                }
            }
            from = lt + 2;
        };
        if escapable && self.interpolates() {
            self.text(close_start, false)?;
        } else {
            self.push_text(content_start, close_start);
        }
        self.pos = close_end;
        let open = self.stack.pop().expect("pushed by start_tag");
        let end_tag = EndTag {
            span: span(close_start, close_end),
            name: span(name_start, name_start + name.len()),
        };
        self.close(&open, Some(end_tag), close_start);
        Ok(())
    }
}

/// `<template lang="pug">`: anything but no `lang`, `lang=""` and `lang="html"`.
fn has_non_html_lang(source: &str, attributes: &[Attribute<'_>]) -> bool {
    attributes.iter().any(|a| {
        a.name.slice(source) == "lang"
            && a.value.is_some_and(|v| !matches!(v.span.slice(source), "" | "html"))
    })
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
            ("<div <span>", Dialect::Html, UnterminatedStartTag, 0),
            ("</div", Dialect::Html, UnterminatedEndTag, 0),
            ("<a b='c>", Dialect::Html, UnterminatedAttributeValue, 5),
            ("<a \"b\">", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<a / b>", Dialect::Html, UnexpectedCharacterInTag, 3),
            ("<script>a", Dialect::Html, UnterminatedRawText, 0),
            ("</div>", Dialect::Html, UnexpectedEndTag, 0),
            ("<div><span></div>", Dialect::Html, UnexpectedEndTag, 11),
            ("<br></br>", Dialect::Html, VoidEndTag, 4),
            ("<template>{{ a</template>", Dialect::Vue, UnterminatedInterpolation, 10),
            ("<template><p><div></p></template>", Dialect::Vue, UnexpectedEndTag, 18),
            ("<docs>a", Dialect::Vue, UnterminatedRawText, 0),
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
