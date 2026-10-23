//! Elements: start and end tags, attributes, raw text content.

use memchr::memchr2;

use oxc_allocator::ArenaBox;

use super::*;

impl<'a> Builder<'a> {
    pub(super) fn end_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        // htm's `<//>` closes the innermost element; its name is the `/`
        let htm = self.rules.htm_closing && self.byte(start + 2) == Some(b'/');
        let name_start = start + 2;
        let name_end = if htm { start + 3 } else { self.tag_name_end(name_start) };
        // As in the HTML tokenizer, the tag runs to its first `>`;
        // what follows the name is ignored (DIVERGENCES.md#an-end-tag-runs-to-its-first-).
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
        if self.tag(name, tags::lowercase(name, &mut buf), self.context().namespace).void {
            return error(DiagnosticKind::VoidEndTag, tag.span);
        }
        // Every dialect so far matches end tags case-insensitively
        let index = if htm {
            self.stack.len().checked_sub(1)
        } else {
            self.stack.iter().rposition(|open| !open.block && open.name.eq_ignore_ascii_case(name))
        };
        let Some(index) = index else {
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        };
        if !self.close_omitted_above(index, start) {
            return error(DiagnosticKind::UnexpectedEndTag, tag.span);
        }
        let open = self.stack.pop().expect("index < len");
        self.close(&open, Some(tag), start);
        Ok(())
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
                // A quote where an attribute name starts is a template's expression leaking out of a value
                // (`content="{{ f("-") }}"`): a printer would re-space it (DIVERGENCES.md#quotes-in-attribute-names-are-an-error).
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
            // Longer than every table entry: lowercased for the formatter's own tables
            let mut key = name_str.to_owned();
            key.make_ascii_lowercase();
            self.allocator.alloc_str(&key)
        } else {
            self.allocator.alloc_str(lowercase)
        };

        // The outermost open element this start tag closes, through omittable ones with `whatwg`
        // (`<li><p>a<li>`, `<tbody><tr><td>a<tbody>`; DIVERGENCES.md#implicit-end-tags-follow-whatwgs-optional-tags).
        let mut closed = None;
        for (index, open) in self.stack.iter().enumerate().rev() {
            if open.tag.closed_by.contains(&key) {
                closed = Some(index);
            }
            if !(self.rules.whatwg && self.omittable(open)) {
                break;
            }
        }
        if let Some(index) = closed {
            self.close_down_to(index, start);
        }

        let inherited = self.context();
        let namespace = tags::namespace(key, inherited.namespace);
        let tag = self.tag(name_str, lowercase, namespace);
        let closes_now = tag.void
            || (self_closing && (self.rules.self_close_any || namespace != Namespace::Html));

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
            // A Vue SFC block is read by Vue's SFC parser, not a browser
            Content::Raw => {
                self.raw_text(tag_span, name_str, false, !sfc_block && lowercase == "script")
            }
            Content::Escapable => self.raw_text(tag_span, name_str, true, false),
        }
    }

    fn attribute(&self, start: usize) -> PResult<Attribute<'a>> {
        // The first character is always part of the name, `=` included, and `<` anywhere (as in browsers,
        // DIVERGENCES.md#-and--are-name-characters); a quote ends it, to be reported.
        let name_end = if self.rules.angular && self.bytes[start] == b'[' {
            self.bracketed_name_end(start)
        } else {
            self.skip(start + 1, |b| !is_attribute_name_end(b))
        };
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
            let mut value_end = self.skip(value_start, |b| !b.is_ascii_whitespace() && b != b'>');
            // Angular's compiler ends it at `/` too, and `<cmp a=b/>` depends on that: the `/` self-closes
            // (DIVERGENCES.md#unquoted-attribute-values-end-at-whitespace-or--only).
            if self.rules.angular
                && value_end > value_start + 1
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
        // As in the HTML tokenizer, the name follows `</` right away
        // (DIVERGENCES.md#raw-text-closes-only-where-the-name-follows-the-slash),
        // and the tag ends as any end tag does.
        let at_name = |at: usize| {
            self.starts_with_ci(at, name) && self.byte(at + name.len()).is_some_and(is_tag_name_end)
        };
        // A script's `</script>` inside `<!-- <script> … -->` is content
        // (DIVERGENCES.md#a-script-holds-script-end-tags-inside----script):
        // 1 inside `<!--`, 2 past a `<script` there, until its `</script`; `-->` ends both.
        let mut escape = 0;
        let mut from = content_start;
        let close_start = loop {
            let rest = &self.bytes[from..];
            let found = if escape > 0 { memchr2(b'<', b'-', rest) } else { memchr(b'<', rest) };
            let Some(at) = found.map(|i| from + i) else {
                return error(DiagnosticKind::UnterminatedRawText, start_tag);
            };
            from = at + 1;
            let rest = &self.bytes[at..];
            if rest.starts_with(b"</") && at_name(at + 2) {
                if escape < 2 {
                    break at;
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
        let name_start = close_start + 2;
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
