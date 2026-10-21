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

    /// The `}}` closing an interpolation, outside string literals;
    /// after a `//` comment, quotes no longer count.
    pub(super) fn interpolation_end(&self, from: usize, limit: usize) -> Option<usize> {
        for at in self.unquoted(from, limit) {
            match &self.bytes[at..limit] {
                [b'}', b'}', ..] => return Some(at),
                [b'/', b'/', ..] => return self.find(at + 2, limit, b"}}"),
                _ => {}
            }
        }
        None
    }

    /// `@name (parameters) {`, or a block without a body; `pos` is at `@`.
    pub(super) fn block_start(&mut self) -> PResult<()> {
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
    pub(super) fn block_close(&mut self) -> PResult<()> {
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
}
