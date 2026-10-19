//! Parser entry point.
//!
//! One pass over the bytes builds the tree directly; there is no token stream.
//! Tree construction is limited to the tag table (`tags.rs`):
//! void elements, raw text, implicit closing.

use oxc_allocator::{Allocator, ArenaBox, ArenaVec};

use crate::ast::*;
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::span::Span;
use crate::{tags, vue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Html,
    /// A Vue SFC.
    /// A file with a root `<html>` or `<!doctype html>` is read with HTML rules instead
    /// (`{{ }}` and directives are still Vue).
    Vue,
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
        let Ok(len) = u32::try_from(self.source.len()) else {
            return Err(Diagnostic::new(DiagnosticKind::SourceTooLong, Span::empty(0)));
        };
        let mut rules = match self.dialect {
            Dialect::Html => Rules { html: true, vue: false, sfc: false },
            Dialect::Vue => Rules { html: false, vue: true, sfc: true },
        };
        loop {
            match Builder::new(self.allocator, self.source, rules, 0, len as usize).run() {
                Ok(children) => {
                    return Ok(Root { children, sfc: rules.sfc, span: Span::new(0, len) });
                }
                Err(Stop::Error(diagnostic)) => return Err(diagnostic),
                Err(Stop::Html) => rules = Rules { html: true, sfc: false, ..rules },
            }
        }
    }
}

/// What a dialect switches on.
#[derive(Clone, Copy)]
struct Rules {
    /// Tag table lookups are case-insensitive, elements close implicitly,
    /// and `/>` closes only void and foreign elements.
    html: bool,
    /// `{{ }}` and directive names.
    vue: bool,
    /// Root elements are SFC blocks: raw text unless `<template>` in HTML.
    sfc: bool,
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

/// HTML ASCII whitespace.
fn is_ws(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

fn is_name_end(b: u8) -> bool {
    is_ws(b) || matches!(b, b'>' | b'<' | b'/' | b'\'' | b'"' | b'=')
}

/// Callers pass offsets inside the source, which `Parser::parse` checked to fit in `u32`.
#[expect(clippy::cast_possible_truncation)]
fn span(start: usize, end: usize) -> Span {
    Span::new(start as u32, end as u32)
}

struct Open<'a> {
    element: Element<'a>,
    /// HTML drops a line break right after the start tag.
    ignore_lf: bool,
    /// Inside `v-pre` (this element or an ancestor).
    v_pre: bool,
}

struct Builder<'a> {
    allocator: &'a Allocator,
    source: &'a str,
    bytes: &'a [u8],
    rules: Rules,
    pos: usize,
    end: usize,
    stack: Vec<Open<'a>>,
    root: ArenaVec<'a, Node<'a>>,
}

impl<'a> Builder<'a> {
    fn new(
        allocator: &'a Allocator,
        source: &'a str,
        rules: Rules,
        pos: usize,
        end: usize,
    ) -> Self {
        Self {
            allocator,
            source,
            bytes: source.as_bytes(),
            rules,
            pos,
            end,
            stack: vec![],
            root: ArenaVec::new_in(&allocator),
        }
    }

    fn run(mut self) -> PResult<ArenaVec<'a, Node<'a>>> {
        while self.pos < self.end {
            if self.at_markup(self.pos) {
                self.markup()?;
            } else {
                self.text(self.end, true)?;
            }
        }
        while let Some(open) = self.stack.pop() {
            self.close(open, None, self.end);
        }
        Ok(self.root)
    }

    // ---
    // Scanning helpers

    fn byte(&self, at: usize) -> Option<u8> {
        if at < self.end { Some(self.bytes[at]) } else { None }
    }

    fn skip_ws(&self, mut at: usize) -> usize {
        while self.byte(at).is_some_and(is_ws) {
            at += 1;
        }
        at
    }

    fn name_end(&self, mut at: usize) -> usize {
        while self.byte(at).is_some_and(|b| !is_name_end(b)) {
            at += 1;
        }
        at
    }

    /// The offset of `pat` in `[from, limit)`.
    fn find(&self, from: usize, limit: usize, pat: &str) -> Option<usize> {
        self.source.get(from..limit)?.find(pat).map(|i| from + i)
    }

    fn starts_with_ci(&self, at: usize, pat: &str) -> bool {
        self.bytes[at..self.end]
            .get(..pat.len())
            .is_some_and(|b| b.eq_ignore_ascii_case(pat.as_bytes()))
    }

    /// `<` followed by what can start markup; any other `<` is text.
    fn at_markup(&self, at: usize) -> bool {
        self.bytes[at] == b'<'
            && self
                .byte(at + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?'))
    }

    /// The name as the tag table sees it.
    fn key<'b>(&self, name: &'b str, buf: &'b mut [u8; 16]) -> &'b str {
        if self.rules.html { tags::lowercase(name, buf) } else { name }
    }

    /// The namespace a new child of the current element starts in.
    fn child_namespace(&self) -> Namespace {
        self.stack.last().map_or(Namespace::Html, |parent| {
            let element = &parent.element;
            if element.namespace == Namespace::Svg
                && element.name.slice(self.source).eq_ignore_ascii_case("foreignObject")
            {
                Namespace::Html
            } else {
                element.namespace
            }
        })
    }

    /// HTML applies its tag table to the HTML namespace only (SVG and MathML elements are never void);
    /// Vue looks void tags up by name alone.
    fn is_void(&self, key: &str, namespace: Namespace) -> bool {
        (namespace == Namespace::Html || !self.rules.html) && tags::is_void(key)
    }

    fn in_v_pre(&self) -> bool {
        self.stack.last().is_some_and(|open| open.v_pre)
    }

    // ---
    // Tree

    fn push(&mut self, node: Node<'a>) {
        match self.stack.last_mut() {
            Some(open) => open.element.children.push(node),
            None => self.root.push(node),
        }
    }

    /// `at` is where the element's content ends when it has no end tag.
    fn close(&mut self, open: Open<'a>, end_tag: Option<Span>, at: usize) {
        let mut element = open.element;
        element.end_tag = end_tag;
        element.span =
            span(element.span.start as usize, end_tag.map_or(at, |tag| tag.end as usize));
        self.push(Node::Element(ArenaBox::new_in(element, &self.allocator)));
    }

    // ---
    // Text

    /// Text from `pos` up to `limit`, or to the next markup when `markup_ends`.
    /// Vue `{{ }}` splits it.
    fn text(&mut self, limit: usize, markup_ends: bool) -> PResult<()> {
        let interpolate = self.rules.vue && !self.in_v_pre();
        let mut start = self.pos;
        while self.pos < limit {
            let b = self.bytes[self.pos];
            if b == b'<' && markup_ends && self.at_markup(self.pos) {
                break;
            }
            if interpolate && b == b'{' && self.byte(self.pos + 1) == Some(b'{') {
                let open = self.pos;
                // Vue closes at the first `}}`, quotes and tags included.
                let Some(close) = self.find(open + 2, limit, "}}") else {
                    return error(DiagnosticKind::UnterminatedInterpolation, span(open, open + 2));
                };
                self.push_text(start, open);
                self.push(Node::Interpolation(Interpolation {
                    span: span(open, close + 2),
                    expression: span(open + 2, close),
                }));
                self.pos = close + 2;
                start = self.pos;
                continue;
            }
            self.pos += 1;
        }
        self.push_text(start, self.pos);
        Ok(())
    }

    fn push_text(&mut self, mut start: usize, end: usize) {
        if let Some(open) = self.stack.last_mut()
            && open.ignore_lf
            && open.element.children.is_empty()
            && !open.element.leading_lf
        {
            let lf = match &self.bytes[start..end] {
                [b'\r', b'\n', ..] => 2,
                [b'\n' | b'\r', ..] => 1,
                _ => 0,
            };
            if lf > 0 {
                start += lf;
                open.element.leading_lf = true;
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
        let text = match bytes.iter().position(|b| !is_ws(*b)) {
            None => Text {
                span: span(start, end),
                content: span(end, end),
                leading: kind(bytes),
                trailing: kind(bytes),
            },
            Some(lead) => {
                let trail = bytes.iter().rposition(|b| !is_ws(*b)).map_or(bytes.len(), |i| i + 1);
                Text {
                    span: span(start, end),
                    content: span(start + lead, start + trail),
                    leading: kind(&bytes[..lead]),
                    trailing: kind(&bytes[trail..]),
                }
            }
        };
        self.push(Node::Text(text));
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
                let Some(close) = self.find(start + 9, self.end, "]]>") else {
                    return error(DiagnosticKind::UnterminatedCdata, span(start, start + 9));
                };
                self.pos = close + 3;
                self.push(Node::Cdata(Cdata { span: span(start, self.pos) }));
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
        let Some(close) = self.find(content_start, self.end, "-->") else {
            return error(DiagnosticKind::UnterminatedComment, span(start, content_start));
        };
        self.pos = close + 3;
        let span = span(start, self.pos);
        let content = self::span(content_start, close);
        let node = self.conditional_comment(span, content).unwrap_or(Node::Comment(Comment {
            span,
            content,
            bogus: false,
        }));
        self.push(node);
        Ok(())
    }

    /// The three IE conditional comment shapes Prettier recognizes.
    fn conditional_comment(&self, span: Span, content: Span) -> Option<Node<'a>> {
        let text = content.slice(self.source);
        let base = content.start as usize;

        let Some(rest) = text.strip_prefix("[if") else {
            let endif =
                text.strip_prefix("<!")?.trim_start_matches(|c: char| c.is_ascii_whitespace());
            return (endif == "[endif]").then_some(Node::ConditionalEnd(ConditionalEnd { span }));
        };
        let close = rest.find(']')?;
        let condition = self::span(base + 3, base + 3 + close);
        let after = rest[close + 1..].strip_prefix('>')?;
        if after == "<!" {
            return Some(Node::ConditionalStart(ConditionalStart { condition, span }));
        }
        let data = after
            .strip_suffix("[endif]")?
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .strip_suffix("<!")?;
        let data_start = base + 3 + close + 2;
        let data_end = data_start + data.len();
        let rules = Rules { sfc: false, ..self.rules };
        let children =
            Builder::new(self.allocator, self.source, rules, data_start, data_end).run().ok();
        Some(Node::ConditionalComment(ArenaBox::new_in(
            ConditionalComment {
                condition,
                content: self::span(data_start, data_end),
                children,
                span,
            },
            &self.allocator,
        )))
    }

    /// `<!x>` / `<?x>`: data up to the next `>`.
    fn bogus_comment(&mut self, prefix: usize) -> PResult<()> {
        let start = self.pos;
        let Some(close) = self.find(start + prefix, self.end, ">") else {
            return error(DiagnosticKind::UnterminatedComment, span(start, start + prefix));
        };
        self.pos = close + 1;
        self.push(Node::Comment(Comment {
            span: span(start, self.pos),
            content: span(start + prefix, close),
            bogus: true,
        }));
        Ok(())
    }

    fn doctype(&mut self) -> PResult<()> {
        let start = self.pos;
        let Some(close) = self.find(start + 9, self.end, ">") else {
            return error(DiagnosticKind::UnterminatedDocType, span(start, start + 9));
        };
        self.pos = close + 1;
        let raw = &self.source[start + 9..close];
        let value_start = start
            + 9
            + (raw.len() - raw.trim_start_matches(|c: char| c.is_ascii_whitespace()).len());
        let value_end = start + 9 + raw.trim_end_matches(|c: char| c.is_ascii_whitespace()).len();
        let value = span(value_start, value_end.max(value_start));
        if self.rules.sfc && self.stack.is_empty() && value.slice(self.source) == "html" {
            return Err(Stop::Html);
        }
        self.push(Node::DocType(DocType { span: span(start, self.pos), value }));
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
        let tag = span(start, self.pos);
        let name = &self.source[name_start..name_end];

        let mut buf = [0; 16];
        if self.is_void(self.key(name, &mut buf), self.child_namespace()) {
            return error(DiagnosticKind::VoidEndTag, tag);
        }
        // Every dialect so far matches end tags case-insensitively.
        let Some(index) = self
            .stack
            .iter()
            .rposition(|open| open.element.name.slice(self.source).eq_ignore_ascii_case(name))
        else {
            return error(DiagnosticKind::UnexpectedEndTag, tag);
        };
        // An element whose `/>` was ignored (`<my-el />`) has no end tag to wait for;
        // the ancestor's end tag closes it, as in browsers.
        let implicitly_closable = |open: &Open<'_>| {
            let mut buf = [0; 16];
            self.rules.html
                && (open.element.self_closing
                    || open.element.namespace == Namespace::Html
                        && tags::closed_by_parent(tags::lowercase(
                            open.element.name.slice(self.source),
                            &mut buf,
                        )))
        };
        if !self.stack[index + 1..].iter().all(implicitly_closable) {
            return error(DiagnosticKind::UnexpectedEndTag, tag);
        }
        while self.stack.len() > index {
            let open = self.stack.pop().expect("index < len");
            let end_tag = (self.stack.len() == index).then_some(tag);
            self.close(open, end_tag, start);
        }
        Ok(())
    }

    fn start_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        let name_end = self.name_end(start + 1);
        let name = span(start + 1, name_end);
        let name_str = name.slice(self.source);
        if self.rules.sfc && self.stack.is_empty() && name_str.eq_ignore_ascii_case("html") {
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
        let key = self.key(name_str, &mut buf);

        if self.rules.html
            && let Some(parent) = self.stack.last()
            && parent.element.namespace == Namespace::Html
        {
            let mut parent_buf = [0; 16];
            let parent_key =
                tags::lowercase(parent.element.name.slice(self.source), &mut parent_buf);
            if tags::closed_by_child(parent_key, key) {
                let open = self.stack.pop().expect("parent exists");
                self.close(open, None, start);
            }
        }

        let namespace = match key {
            "svg" => Namespace::Svg,
            "math" => Namespace::MathMl,
            _ => self.child_namespace(),
        };
        let html = namespace == Namespace::Html;
        let void = self.is_void(key, namespace);
        let closes_now = void || (self_closing && (!self.rules.html || !html));
        let ignore_lf = html && tags::ignores_first_lf(key);

        let v_pre = self.rules.vue
            && (self.in_v_pre() || attributes.iter().any(|a| a.name.slice(self.source) == "v-pre"));
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
            tags::Content::Normal
        } else if self.rules.sfc && self.stack.is_empty() {
            if name_str == "template" && !has_non_html_lang(self.source, &attributes) {
                tags::Content::Normal
            } else {
                tags::Content::Raw
            }
        } else {
            let mut lower_buf = [0; 16];
            tags::content(tags::lowercase(name_str, &mut lower_buf))
        };

        let tag_span = span(start, at);
        let open = Open {
            element: Element {
                name,
                namespace,
                attributes,
                children: ArenaVec::new_in(&self.allocator),
                start_tag: tag_span,
                self_closing,
                leading_lf: false,
                end_tag: None,
                span: tag_span,
            },
            ignore_lf,
            v_pre,
        };
        if closes_now {
            self.close(open, None, at);
            return Ok(());
        }
        self.stack.push(open);
        match content {
            tags::Content::Normal => Ok(()),
            tags::Content::Raw => self.raw_text(tag_span, name_str, false),
            tags::Content::Escapable => self.raw_text(tag_span, name_str, true),
        }
    }

    fn attribute(&self, start: usize) -> PResult<Attribute<'a>> {
        // The first character is always part of the name, `=` included (as in browsers).
        let name_end = self.name_end(start + 1);
        let name = span(start, name_end);
        let mut end = name_end;
        let mut value = None;

        let eq = self.skip_ws(name_end);
        if self.byte(eq) == Some(b'=') {
            let value_start = self.skip_ws(eq + 1);
            let (quote, pat) = match self.byte(value_start) {
                Some(b'"') => (Quote::Double, "\""),
                Some(b'\'') => (Quote::Single, "'"),
                _ => (Quote::None, ""),
            };
            if quote == Quote::None {
                // Browsers (and Vue) end an unquoted value at whitespace or `>` only:
                // `href=/a/b` and `src=a.png/` keep their slashes.
                let mut value_end = value_start;
                while self.byte(value_end).is_some_and(|b| !is_ws(b) && b != b'>') {
                    value_end += 1;
                }
                value = Some(AttributeValue { span: span(value_start, value_end), quote });
                end = value_end;
            } else {
                let Some(close) = self.find(value_start + 1, self.end, pat) else {
                    return error(
                        DiagnosticKind::UnterminatedAttributeValue,
                        span(value_start, self.end),
                    );
                };
                value = Some(AttributeValue { span: span(value_start + 1, close), quote });
                end = close + 1;
            }
        }
        Ok(Attribute { name, value, kind: AttributeKind::Plain, span: span(start, end) })
    }

    /// Content up to `</name>` (case-insensitive) as one text, `{{ }}` split when `escapable`.
    fn raw_text(&mut self, start_tag: Span, name: &str, escapable: bool) -> PResult<()> {
        let content_start = self.pos;
        let mut from = content_start;
        let (close_start, close_end) = loop {
            let Some(lt) = self.find(from, self.end, "</") else {
                return error(DiagnosticKind::UnterminatedRawText, start_tag);
            };
            let name_start = self.skip_ws(lt + 2);
            if self.starts_with_ci(name_start, name) {
                let gt = self.skip_ws(name_start + name.len());
                if self.byte(gt) == Some(b'>') {
                    break (lt, gt + 1);
                }
            }
            from = lt + 2;
        };
        if escapable {
            self.text(close_start, false)?;
        } else {
            self.push_text(content_start, close_start);
        }
        self.pos = close_end;
        let open = self.stack.pop().expect("pushed by start_tag");
        self.close(open, Some(span(close_start, close_end)), close_start);
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
        let [Node::ConditionalComment(comment)] = root.children.as_slice() else { panic!() };
        assert!(comment.children.is_none());
    }
}
