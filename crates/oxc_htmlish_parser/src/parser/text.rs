//! Text, split at `{{ }}` (and Angular blocks, `@let` and ICU expressions).

use memchr::{memchr2, memrchr};

use super::*;

impl Builder<'_> {
    /// Text from `pos` up to `limit`, or to the next markup when `markup_ends`.
    /// `{{ }}`, Angular blocks, `@let` and ICU expressions split it.
    pub(super) fn text(&mut self, limit: usize, markup_ends: bool) -> PResult<()> {
        let interpolate = self.interpolates();
        let angular = self.rules.angular && markup_ends;
        let mut start = self.pos;
        while self.pos < limit {
            let haystack = &self.bytes[self.pos..limit];
            let found = if angular {
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
                // Not in an ICU case (DIVERGENCES.md#inside-an-icu-case)
                // Under `ngNonBindable` read as Angular reads them, but text, joining the text around (`Builder::push_text`)
                b'@' if angular
                    && !self.context().icu
                    && (self.at_let(self.pos) || self.at_block(self.pos)) =>
                {
                    self.push_text(start, self.pos);
                    let at = self.pos;
                    if self.at_let(self.pos) {
                        self.let_declaration()?;
                    } else {
                        self.block_start()?;
                    }
                    start = if self.context().non_bindable { at } else { self.pos };
                    continue;
                }
                b'}' if angular => {
                    self.push_text(start, self.pos);
                    let at = self.pos;
                    // Of the block open at the `}`, which may be outside `ngNonBindable` (`@if (a) {<p ngNonBindable>}`)
                    let text = self.innermost_block_is_text();
                    self.block_close()?;
                    start = if text { at } else { self.pos };
                    continue;
                }
                b'@' => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            if self.byte(self.pos + 1) != Some(b'{')
                || (!markup_ends && self.rules.sfc && self.after_end_tag_prefix(self.pos))
            {
                // In Angular a `{` that opens no interpolation opens an ICU expression
                if angular {
                    self.push_text(start, self.pos);
                    self.icu_start()?;
                    start = self.pos;
                } else {
                    self.pos += 1;
                }
                continue;
            }
            let open = self.pos;
            let close = if self.rules.angular {
                match angular::interpolation_end(self.bytes, open + 2, limit) {
                    Ok(close) => close,
                    Err(stop) => {
                        // Angular reads an unterminated `{{` as text, up to where the search stopped
                        self.pos = stop;
                        continue;
                    }
                }
            } else {
                self.vue_interpolation_end(open, limit)?
            };
            if self.context().non_bindable {
                self.pos = close + 2;
                continue;
            }
            self.push_text(start, open);
            self.push(
                NodeKind::Interpolation(Interpolation {
                    angular: if self.rules.angular {
                        angular_expression::expression(
                            self.allocator,
                            self.source,
                            span(open + 2, close),
                        )
                    } else {
                        None
                    },
                }),
                span(open, close + 2),
            );
            self.pos = close + 2;
            start = self.pos;
        }
        self.push_text(start, self.pos);
        Ok(())
    }

    /// Vue closes an interpolation `{{` at `open` opens at its first `}}`, quotes and tags included.
    pub(super) fn vue_interpolation_end(&self, open: usize, limit: usize) -> PResult<usize> {
        self.find(open + 2, limit, b"}}").map_or_else(
            || error(DiagnosticKind::UnterminatedInterpolation, span(open, open + 2)),
            Ok,
        )
    }

    /// Vue, in RCDATA: whether the text before `at` ends in part of the element's end tag (`<`, `</ti`).
    /// Vue's tokenizer reads the character that breaks the match (a `{` here) as text, unread for `{{` (`<{{ t }}` is text).
    pub(super) fn after_end_tag_prefix(&self, at: usize) -> bool {
        let Some(open) = self.stack.last() else { return false };
        let name = open.name.as_bytes();
        // A name holds no `<`, so only the last one before `at` can start the match
        let window = &self.bytes[at.saturating_sub(name.len() + 1)..at];
        memrchr(b'<', window).is_some_and(|i| {
            let seen = &window[i + 1..];
            seen.first().is_none_or(|&b| b == b'/')
                && seen.get(1..).is_none_or(|rest| rest.eq_ignore_ascii_case(&name[..rest.len()]))
        })
    }

    pub(super) fn push_text(&mut self, mut start: usize, end: usize) {
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

        // Under `ngNonBindable`, joined to the text it follows, read apart at a block's marker or `@let`
        let parent = self.stack.last().map_or(self.base_parent, |open| open.id);
        let prev = self.nodes[parent.index()].last_child;
        if self.context().non_bindable
            && let Some(prev) = prev
            && matches!(self.nodes[prev.index()].kind, NodeKind::Text(_))
            && self.nodes[prev.index()].span.end as usize == start
        {
            let joined = self.nodes[prev.index()].span.start as usize;
            let text = self.text_of(joined, end);
            let node = &mut self.nodes[prev.index()];
            node.kind = NodeKind::Text(text);
            node.span = span(joined, end);
            return;
        }
        let text = self.text_of(start, end);
        self.push(NodeKind::Text(text), span(start, end));
    }

    /// A text node's facts for the source in `[start, end)`.
    fn text_of(&self, start: usize, end: usize) -> Text {
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
        Text {
            content: span(start + lead, start + lead.max(trail)),
            leading: kind(&bytes[..lead]),
            trailing: kind(&bytes[trail..]),
        }
    }
}
