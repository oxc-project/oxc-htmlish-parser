//! Angular: `{{ }}` closed outside string literals, and control flow blocks (`@if (a) { … }`).

use oxc_allocator::ArenaBox;

use super::*;

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

impl<'a> Builder<'a> {
    /// An Angular block starts at `at`.
    pub(super) fn at_block(&self, at: usize) -> bool {
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
                let (len, outside) = string_step(self.bytes[i], &mut quote, true);
                at += len;
                if outside {
                    return Some(i);
                }
            }
            None
        })
    }

    /// The `}}` closing an interpolation, outside string literals; after a `//` comment, quotes no longer count.
    /// `Err` with where the search stopped when a tag starts first, even in a string, or the input ends:
    /// Angular reads what it searched as text.
    /// As in Angular's lexer, the character right after `//` is read without looking for a tag (`{{ a //<b }}` holds no tag).
    pub(super) fn interpolation_end(&self, from: usize, limit: usize) -> Result<usize, usize> {
        let (mut quote, mut comment) = (None, false);
        let mut at = from;
        while at < limit {
            if self.at_tag_start(at) {
                return Err(at);
            }
            if quote.is_none() {
                match &self.bytes[at..limit] {
                    [b'}', b'}', ..] => return Ok(at),
                    [b'/', b'/', ..] => {
                        comment = true;
                        at += 2;
                    }
                    _ => {}
                }
            }
            let Some(&b) = self.bytes[..limit].get(at) else { break };
            at += string_step(b, &mut quote, !comment).0;
        }
        Err(at.min(limit))
    }

    /// `[class.[&>svg]:w-4]`: a name opened with `[` runs while its brackets are open, up to a line break
    /// (Angular's lexer, for Tailwind's arbitrary variants; DIVERGENCES.md#bracketed-attribute-names).
    pub(super) fn bracketed_name_end(&self, start: usize) -> usize {
        let (mut open, mut at) = (0, start);
        while let Some(b) = self.byte(at) {
            match b {
                b'[' => open += 1,
                b']' => open -= 1,
                _ => {}
            }
            let end = if open > 0 {
                matches!(b, b'\n' | b'\r')
            } else {
                self.ws_len(at) > 0 || ends_attribute_name(b)
            };
            if end {
                break;
            }
            at += 1;
        }
        at
    }

    /// `@name (parameters) {`, or a block without a body; `pos` is at `@`.
    pub(super) fn block_start(&mut self) -> PResult<()> {
        let start = self.pos;
        // The name may hold whitespace after its first character (`else if`), Angular's whitespace included
        let name_start = start + 1;
        let (mut at, mut name_end) = (name_start, name_start);
        loop {
            match self.byte(at) {
                Some(b) if b.is_ascii_alphanumeric() || b == b'_' => {
                    at += 1;
                    name_end = at;
                }
                _ => match self.ws_len(at) {
                    0 => break,
                    n => at += n,
                },
            }
        }
        let mut parameters = ArenaVec::new_in(&self.allocator);
        let mut open_end = name_end;
        if self.byte(at) == Some(b'(') {
            at = self.block_parameters(at + 1, &mut parameters)?;
            open_end = at;
            at = self.skip_ws(at);
        }
        let name = &self.source[name_start..name_end];
        let body = self.byte(at) == Some(b'{');
        // `@default never;` has no body;
        // cases fall through to the next one without one.
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
                // Each parameter counts its own parentheses, as Angular's lexer does
                b';' => {
                    push(param_start, at, out);
                    param_start = at + 1;
                    depth = 0;
                }
                _ => {}
            }
        }
        error(DiagnosticKind::IncompleteBlock, span(from - 1, self.bytes.len()))
    }

    /// `}`: closes the innermost block, and the elements in it whose end tag may be omitted.
    pub(super) fn block_close(&mut self) -> PResult<()> {
        let at = self.pos;
        let index = self.stack.iter().rposition(|open| open.block);
        if !index.is_some_and(|index| self.close_omitted_above(index + 1, at)) {
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
}

/// One step through Angular expression text: string literals open at `'`, `"` and `` ` `` (when `opens`),
/// and a `\\` escapes the next byte.
/// Returns the bytes the step takes, and whether `b` lies outside any string (and is no escape).
fn string_step(b: u8, quote: &mut Option<u8>, opens: bool) -> (usize, bool) {
    match (b, *quote) {
        (b'\\', _) => (2, false),
        (b, Some(q)) if b == q => {
            *quote = None;
            (1, false)
        }
        (_, Some(_)) => (1, false),
        (q @ (b'\'' | b'"' | b'`'), None) if opens => {
            *quote = Some(q);
            (1, false)
        }
        _ => (1, true),
    }
}
