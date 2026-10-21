//! Elements: start and end tags, attributes, raw text content.

use oxc_allocator::ArenaBox;

use super::*;

impl<'a> Builder<'a> {
    pub(super) fn end_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        // htm's `<//>` closes the innermost element; its name is the `/`.
        let htm = self.rules.htm_closing && self.byte(start + 2) == Some(b'/');
        let (name_start, name_end) = if htm {
            (start + 2, start + 3)
        } else {
            let name_start = self.skip_ws(start + 2);
            (name_start, self.name_end(name_start))
        };
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
            // As in the HTML tokenizer, the name follows `</` right away (DIVERGENCES.md#raw-text-closes-only-where-the-name-follows-the-slash).
            let name_start = lt + 2;
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
