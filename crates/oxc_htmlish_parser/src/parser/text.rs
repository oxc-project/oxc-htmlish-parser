//! Text, split at `{{ }}` (and Angular blocks).

use memchr::memchr2;

use super::*;

impl Builder<'_> {
    /// Text from `pos` up to `limit`, or to the next markup when `markup_ends`.
    /// `{{ }}` and Angular blocks split it.
    pub(super) fn text(&mut self, limit: usize, markup_ends: bool) -> PResult<()> {
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
