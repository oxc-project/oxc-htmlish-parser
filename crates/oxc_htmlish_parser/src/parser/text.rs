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
                // Under `ngNonBindable` read as Angular reads them, but text: the run goes on
                b'@' if angular
                    && !self.context().icu
                    && (self.at_let(self.pos) || self.at_block(self.pos)) =>
                {
                    let text = self.context().non_bindable;
                    if !text {
                        self.push_text(start, self.pos);
                    }
                    if self.at_let(self.pos) {
                        self.let_declaration()?;
                    } else {
                        self.block_start()?;
                    }
                    if !text {
                        start = self.pos;
                    }
                    continue;
                }
                b'}' if angular => {
                    let at = self.pos;
                    // A block read as text closes in the run, but for the elements left open in it, which close at the `}`
                    let in_text = self
                        .stack
                        .last()
                        .is_some_and(|open| matches!(open.braced, Braced::Text(_)));
                    if !in_text {
                        self.push_text(start, at);
                    }
                    let text = self.block_close()?;
                    if !text {
                        start = self.pos;
                    } else if !in_text {
                        start = at;
                    }
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
                match angular::interpolation_end(self.bytes, open + 2, limit, None) {
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
                            true,
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
}

impl Interpolation<'_> {
    /// A character reference in `expression` decodes to a quote as `dialect` reads it (`'&quot;'` is `'"'` there):
    /// Angular's needs its `;`, Vue's is HTML's in a text (`&quot`, `&#39` too); no other dialect reads a `{{ }}`.
    pub fn decodes_quote(expression: &str, dialect: Dialect) -> bool {
        let bytes = expression.as_bytes();
        memchr::memchr_iter(b'&', bytes).any(|at| {
            let char = match dialect {
                Dialect::Angular => {
                    angular_expression::character_reference(&bytes[at..]).map(|(char, _)| char)
                }
                Dialect::Vue => html_quote_reference(&bytes[at..]),
                Dialect::Html | Dialect::Xhtml => None,
            };
            matches!(char, Some(b'"' | b'\''))
        })
    }
}

/// The quote the character reference `bytes` starts with decodes to in an HTML text, as Vue decodes one (`decodeHTML`):
/// `&quot` and `&QUOT` with or without their `;` (legacy names), `&apos;`, a number (`&#39`, `&#x22;`) with or without its `;`.
fn html_quote_reference(bytes: &[u8]) -> Option<u8> {
    let rest = &bytes[1..];
    if rest.starts_with(b"quot") || rest.starts_with(b"QUOT") {
        return Some(b'"');
    }
    if rest.starts_with(b"apos;") {
        return Some(b'\'');
    }
    let number = rest.strip_prefix(b"#")?;
    let (digits, radix) = match number.strip_prefix(b"x").or_else(|| number.strip_prefix(b"X")) {
        Some(hex) => (hex, 16),
        None => (number, 10),
    };
    let len = digits.iter().take_while(|b| char::from(**b).is_digit(radix)).count();
    let code = u32::from_str_radix(std::str::from_utf8(&digits[..len]).ok()?, radix).ok()?;
    match code {
        0x22 => Some(b'"'),
        0x27 => Some(b'\''),
        _ => None,
    }
}
