//! Markup other than elements: comments (IE conditional ones included), CDATA, doctype.

use oxc_allocator::ArenaBox;

use super::*;

impl Builder<'_> {
    pub(super) fn markup(&mut self) -> PResult<()> {
        let start = self.pos;
        match self.bytes[start + 1] {
            // As in the HTML tokenizer, `</` without a name right after it is a bogus comment (`</ div>`);
            // htm's `<//>` aside (DIVERGENCES.md#-without-a-name-right-after-it-is-a-bogus-comment).
            b'/' if self.byte(start + 2).is_some_and(|b| {
                !b.is_ascii_alphabetic() && b != b'>' && !(b == b'/' && self.rules.htm_closing)
            }) =>
            {
                self.bogus_comment(2)
            }
            b'/' => self.end_tag(),
            b'!' if self.starts_with_ci(start, "<!--") => self.comment(),
            b'!' if self.bytes[start..].starts_with(b"<![CDATA[") => {
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
        // `<!-->` and `<!--->` are empty comments, as in the HTML tokenizer (DIVERGENCES.md#a-comment-may-close-right-after-its-opening).
        let (close, end) = match &self.bytes[content_start..] {
            [b'>', ..] => (content_start, content_start + 1),
            [b'-', b'>', ..] => (content_start, content_start + 2),
            _ => {
                let Some(close) = self.find(content_start, self.bytes.len(), b"-->") else {
                    return error(DiagnosticKind::UnterminatedComment, span(start, content_start));
                };
                (close, close + 3)
            }
        };
        self.pos = end;
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
}
