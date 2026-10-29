//! Elements: start and end tags, attributes, raw text content.

use memchr::memchr2;

use oxc_allocator::ArenaBox;

use super::*;

impl<'a> Builder<'a> {
    pub(super) fn end_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        // htm's `<//>` closes the innermost element; its name is the `/`
        let htm = self.rules.htm_closing && self.byte(start + 2) == Some(b'/');
        let name_start = self.end_tag_name_start(start);
        let name_end = if htm { start + 3 } else { self.tag_name_end(name_start) };
        // As in the HTML tokenizer, the tag runs to its first `>`;
        // what follows the name is ignored (DIVERGENCES.md#end-tag-with-attributes).
        let Some(gt) = self.find(name_end, self.bytes.len(), b">") else {
            return error(DiagnosticKind::UnterminatedEndTag, span(start, self.bytes.len()));
        };
        self.pos = gt + 1;
        let tag = EndTag {
            span: span(start, self.pos),
            name: span(name_start, name_end),
            ignored: self.ignored(name_end, gt),
        };
        let name = &self.source[name_start..name_end];

        let mut buf = [0; 16];
        let lowercase = tags::lowercase(name, &mut buf);
        if self.tag(name, lowercase, self.context().namespace, false).void {
            return error(DiagnosticKind::VoidEndTag, tag.span);
        }
        // Every dialect so far matches end tags case-insensitively
        let index = if htm {
            self.stack.len().checked_sub(1)
        } else if self.is_template_placeholder(name) {
            self.stack
                .iter()
                .rposition(|open| !open.block && self.is_template_placeholder(open.name))
        } else {
            self.stack.iter().rposition(|open| !open.block && open.name.eq_ignore_ascii_case(name))
        };
        let Some(index) = index else {
            if self.omitted_element_end(tag, lowercase) {
                return Ok(());
            }
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        };
        if !self.close_omitted_above(index + 1, start) {
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        }
        let open = self.stack.pop().expect("index < len");
        self.close(&open, Some(tag), start);
        Ok(())
    }

    /// The end tag of an `html`, `head` or `body` (`name`, lowercase) whose start tag is omitted,
    /// where WHATWG reads it without an error: a node of its own after the content
    /// (DIVERGENCES.md#end-tags-of-omitted-html-head-and-body).
    #[inline(never)]
    fn omitted_element_end(&mut self, tag: EndTag, name: &str) -> bool {
        if !self.rules.whatwg
            || self.base_parent != NodeId::DOCUMENT
            || !matches!(name, "html" | "head" | "body")
        {
            return false;
        }
        // A written `<html>` holds `head` and `body`
        let (keep, parent) = match self.stack.first() {
            Some(open) if name != "html" && open.name.eq_ignore_ascii_case("html") => (1, open.id),
            _ => (0, NodeId::DOCUMENT),
        };
        let mut before = std::iter::successors(self.nodes[parent.index()].first_child, |&id| {
            self.nodes[id.index()].next
        })
        .map(|id| &self.nodes[id.index()]);
        let valid = if name == "head" {
            // Nothing of the body yet
            self.stack.len() == keep
                && before.all(|node| match &node.kind {
                    NodeKind::Text(text) => text.is_whitespace_only(),
                    NodeKind::Element(el) => tags::is_head_content(el.key),
                    NodeKind::Comment(_)
                    | NodeKind::ConditionalComment(_)
                    | NodeKind::ConditionalStart(_)
                    | NodeKind::ConditionalEnd
                    | NodeKind::DocType(_) => true,
                    _ => false,
                })
        } else {
            // Once, before what follows the element's end; the open elements whose end tags may go close here
            let after: &[&str] = if name == "html" { &["html"] } else { &["html", "body"] };
            !before.any(|node| match &node.kind {
                NodeKind::Element(el) => after.contains(&el.key),
                NodeKind::EndTag(end) => {
                    after.iter().any(|n| end.name.slice(self.source).eq_ignore_ascii_case(n))
                }
                _ => false,
            }) && self.close_omitted_above(keep, tag.span.start as usize)
        };
        if valid {
            self.push(NodeKind::EndTag(ArenaBox::new_in(tag, &self.allocator)), tag.span);
        }
        valid
    }

    pub(super) fn start_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        let name_end = self.tag_name_end(start + 1);
        let name = span(start + 1, name_end);
        let name_str = name.slice(self.source);
        if self.at_sfc_root() && name_str.eq_ignore_ascii_case("html") {
            return Err(Stop::Html);
        }

        let mut attributes = ArenaVec::new_in(&self.allocator);
        let mut at = self.skip_ws(name_end);
        let self_closing = loop {
            match self.byte(at) {
                None => return error(DiagnosticKind::UnterminatedStartTag, span(start, at)),
                Some(b'>') => {
                    at += 1;
                    break false;
                }
                Some(b'/') if self.byte(at + 1) == Some(b'>') => {
                    at += 2;
                    break true;
                }
                // Vue's SFC tokenizer ends a start tag at `</` where an attribute could start (`<C a="b"</C>`)
                Some(b'<') if self.rules.sfc && self.byte(at + 1) == Some(b'/') => break false,
                // A quote where an attribute name starts is a template's expression leaking out of a value
                // (`content="{{ f("-") }}"`): a printer would re-space it (DIVERGENCES.md#quotes-in-attribute-names).
                // Vue's SFC tokenizer reads it as the name's first character.
                Some(b) if b == b'/' || (!self.rules.sfc && matches!(b, b'"' | b'\'')) => {
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

        // Angular reads a namespace prefix as its compiler does: `svg:rect` is an SVG `rect` (DIVERGENCES.md#namespace-prefixes)
        let (local, prefixed) = match self.angular_prefix(name_str) {
            Some((prefix, local)) => (local, Some(tags::namespace(prefix, Namespace::Html))),
            None => (name_str, None),
        };
        let mut buf = [0; 16];
        let lowercase = tags::lowercase(local, &mut buf);
        let key = if !self.rules.html || !local.bytes().any(|b| b.is_ascii_uppercase()) {
            local
        } else if lowercase.is_empty() {
            // Longer than every table entry: lowercased for the formatter's own tables
            let mut key = local.to_owned();
            key.make_ascii_lowercase();
            self.allocator.alloc_str(&key)
        } else {
            self.allocator.alloc_str(lowercase)
        };

        // The outermost open element this start tag closes, through omittable ones with `whatwg`
        // (`<li><p>a<li>`, `<tbody><tr><td>a<tbody>`; DIVERGENCES.md#implicit-end-tags).
        let mut closed = None;
        let mut end = self.stack.len();
        while let Some(index) = self.stack[..end].last().and_then(|open| open.look) {
            let open = &self.stack[index];
            if open.tag.closed_by.contains(&key) {
                closed = Some(index);
            }
            if !(self.rules.whatwg && open.omittable) {
                break;
            }
            end = index;
        }
        if let Some(index) = closed {
            self.close_down_to(index, start);
        }

        let inherited = self.context();
        let namespace = prefixed.unwrap_or_else(|| tags::namespace(key, inherited.namespace));
        let tag = self.tag(name_str, lowercase, namespace, prefixed == Some(Namespace::Svg));
        let closes_now = tag.void
            || (self_closing
                && (self.rules.self_close_any
                    || namespace != Namespace::Html
                    || self.is_template_placeholder(name_str)));

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

        let sfc_block = self.at_sfc_root();
        let content = if closes_now {
            Content::Normal
        } else if sfc_block {
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
        let omittable = tag.closed_by_parent || self_closing;
        let passed = self.rules.whatwg && omittable && tag.closed_by.is_empty();
        let look = if passed {
            self.stack.last().and_then(|below| below.look)
        } else {
            Some(self.stack.len())
        };
        let open = Open {
            id,
            name: name_str,
            block: false,
            tag,
            omittable,
            look,
            context: Context { namespace: tags::children_namespace(local, namespace), v_pre },
        };
        if closes_now {
            self.close(&open, None, at);
            return Ok(());
        }
        self.stack.push(open);
        match content {
            Content::Normal => Ok(()),
            // A Vue SFC block is read by Vue's SFC parser, and Angular's by its compiler, not a browser
            Content::Raw => {
                let script = !sfc_block && !self.rules.angular && lowercase == "script";
                self.raw_text(tag_span, local, false, script)
            }
            Content::Escapable => self.raw_text(tag_span, local, true, false),
        }
    }

    fn attribute(&self, start: usize) -> PResult<Attribute<'a>> {
        // The first character is part of the name, `=` included, and `<` anywhere (as in browsers, DIVERGENCES.md#name-characters),
        // but in Angular, whose compiler reads a name it ends at once, empty (`<a =b>`; DIVERGENCES.md#equals-sign-before-an-attribute-name).
        // A quote ends it, to be reported, but in a Vue directive name, which takes quotes as Vue reads them
        // (DIVERGENCES.md#quotes-in-attribute-names).
        let name_end = if self.rules.angular && self.bytes[start] == b'[' {
            self.bracketed_name_end(start)
        } else if self.rules.angular && self.bytes[start] == b'=' {
            start
        } else if self.rules.sfc && vue::is_directive_start(&self.bytes[start..]) {
            self.until_ws_or(start + 1, |b| matches!(b, b'>' | b'/' | b'='))
        } else {
            self.until_ws_or(start + 1, ends_attribute_name)
        };
        // A quote right after a name is in it, to be reported, as in every oracle
        if matches!(self.byte(name_end), Some(b'"' | b'\'')) {
            return error(DiagnosticKind::UnexpectedCharacterInTag, span(name_end, name_end + 1));
        }
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
            // Browsers (and Vue) end an unquoted value at whitespace or `>` only: `href=/a/b` and `src=a.png/` keep their slashes.
            // Angular's compiler ends it at `=` too (`b=c=d`), and at a `/` right before `>`, which self-closes (`<cmp a=b/>`;
            // DIVERGENCES.md#unquoted-attribute-values).
            let mut value_end = if self.rules.angular {
                self.until_ws_or(value_start, |b| matches!(b, b'>' | b'='))
            } else {
                self.until_ws_or(value_start, |b| b == b'>')
            };
            if self.rules.angular
                && value_end > value_start
                && self.bytes[value_end - 1] == b'/'
                && self.byte(value_end) == Some(b'>')
            {
                value_end -= 1;
            }
            (AttributeValue { span: span(value_start, value_end), quote: Quote::None }, value_end)
        };
        Ok(Attribute {
            name,
            value: Some(value),
            kind: AttributeKind::Plain,
            span: span(start, end),
        })
    }

    /// Content up to `</name>` (case-insensitive) as one text, `{{ }}` split when `escapable`;
    /// `script`: with the HTML tokenizer's script data escapes.
    fn raw_text(
        &mut self,
        start_tag: Span,
        name: &str,
        escapable: bool,
        script: bool,
    ) -> PResult<()> {
        let content_start = self.pos;
        let at_name = |at: usize| {
            self.starts_with_ci(at, name) && self.byte(at + name.len()).is_some_and(is_tag_name_end)
        };
        // Where the name of an end tag `</` at `at` closes starts: as in the HTML tokenizer, right after `</`,
        // and the tag ends as any end tag does; in Angular, whitespace may surround the name and `>` follows,
        // as its compiler reads it (DIVERGENCES.md#raw-text-end-tag).
        let closes = |at: usize| {
            if !self.rules.angular {
                return at_name(at + 2).then_some(at + 2);
            }
            let name_start = self.skip_ws(at + 2);
            (self.starts_with_ci(name_start, name)
                && self.byte(self.skip_ws(name_start + name.len())) == Some(b'>'))
            .then_some(name_start)
        };
        // A script's `</script>` inside `<!-- <script> … -->` is content
        // (DIVERGENCES.md#script-data-escapes):
        // 1 inside `<!--`, 2 past a `<script` there, until its `</script`; `-->` ends both.
        let mut escape = 0;
        // Vue reads an interpolation in RCDATA up to its `}}`, past the end tag too
        let vue_interpolation = escapable && self.rules.sfc && self.interpolates();
        let mut from = content_start;
        let (close_start, name_start) = loop {
            let rest = &self.bytes[from..];
            let found = if escape > 0 {
                memchr2(b'<', b'-', rest)
            } else if vue_interpolation {
                memchr2(b'<', b'{', rest)
            } else {
                memchr(b'<', rest)
            };
            let Some(at) = found.map(|i| from + i) else {
                return error(DiagnosticKind::UnterminatedRawText, start_tag);
            };
            from = at + 1;
            let rest = &self.bytes[at..];
            if vue_interpolation && rest.starts_with(b"{{") && !self.after_end_tag_prefix(at) {
                from = self.vue_interpolation_end(at, self.bytes.len())? + 2;
            } else if rest.starts_with(b"</")
                && let Some(name_start) = closes(at)
            {
                if escape < 2 {
                    break (at, name_start);
                }
                escape = 1;
            } else if script && escape == 0 && rest.starts_with(b"<!--") {
                escape = 1;
                // Its dashes may end it: `<!-->`.
                from = at + 2;
            } else if escape > 0 && rest.starts_with(b"-->") {
                escape = 0;
                from = at + 3;
            } else if escape == 1 && rest[0] == b'<' && at_name(at + 1) {
                escape = 2;
            }
        };
        let name_end = name_start + name.len();
        // Without a `>` here, no later end tag has one either.
        let Some(gt) = self.find(name_end, self.bytes.len(), b">") else {
            return error(DiagnosticKind::UnterminatedRawText, start_tag);
        };
        let close_end = gt + 1;
        if escapable && self.interpolates() {
            self.text(close_start, false)?;
        } else {
            self.push_text(content_start, close_start);
        }
        self.pos = close_end;
        let open = self.stack.pop().expect("pushed by start_tag");
        let end_tag = EndTag {
            span: span(close_start, close_end),
            name: span(name_start, name_end),
            ignored: self.ignored(name_end, close_end - 1),
        };
        self.close(&open, Some(end_tag), close_start);
        Ok(())
    }

    /// Angular: `name`'s namespace prefix and local name, as its compiler splits them:
    /// a prefix is the letters and digits before the first `:`.
    fn angular_prefix<'n>(&self, name: &'n str) -> Option<(&'n str, &'n str)> {
        if !self.rules.angular {
            return None;
        }
        let (prefix, local) = name.split_once(':')?;
        (!prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then_some((prefix, local))
    }

    /// `[from, gt)` trimmed, when anything is left.
    fn ignored(&self, from: usize, gt: usize) -> Option<Span> {
        let (lead, trail) = trim(&self.bytes[from..gt]);
        (lead < trail).then(|| span(from + lead, from + trail))
    }
}

/// `<template lang="pug">`: anything but no `lang`, `lang=""` and `lang="html"`.
fn has_non_html_lang(source: &str, attributes: &[Attribute<'_>]) -> bool {
    attributes.iter().any(|a| {
        a.name.slice(source) == "lang"
            && a.value.is_some_and(|v| !matches!(v.span.slice(source), "" | "html"))
    })
}
