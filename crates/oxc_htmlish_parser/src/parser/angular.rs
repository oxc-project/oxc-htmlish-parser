//! Angular: `{{ }}` closed outside string literals, control flow blocks (`@if (a) { … }`), `@let`,
//! ICU expressions (`{n, plural, =0 {none}}`), and binding and i18n attribute names (`[x]`, `i18n-x`).

use oxc_allocator::ArenaBox;

use super::*;

/// The block names Angular's lexer starts a block at, by prefix (`@iffy` is a block named `iffy`).
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

    /// `@let` starts at `at`, even without the whitespace after it (`@letter` is an incomplete one).
    pub(super) fn at_let(&self, at: usize) -> bool {
        self.bytes[at..].starts_with(b"@let")
    }

    /// `@let`, or else a block, as Angular's lexer tries them; `pos` is at `@`.
    pub(super) fn let_or_block(&mut self) -> PResult<()> {
        if self.at_let(self.pos) { self.let_declaration() } else { self.block_start() }
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
        let reading = control_flow::Reading::of(name);
        let parameters = ArenaVec::from_iter_in(
            parameters.iter().enumerate().map(|(index, &parameter)| {
                control_flow::parameter(self.allocator, self.source, reading, index, parameter)
            }),
            &self.allocator,
        );
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
            self.open_container(id, false);
        }
        Ok(())
    }

    /// Opens a block, an ICU expression or a case at `id`; `icu`: its content is a case's.
    fn open_container(&mut self, id: NodeId, icu: bool) {
        let context = Context { icu, ..self.context() };
        self.stack.push(Open {
            id,
            name: "",
            block: true,
            tag: Tag::default(),
            omittable: false,
            look: Some(self.stack.len()),
            context,
        });
    }

    /// `{expression, type,`, then its first case; `pos` is at `{`.
    /// As Angular's lexer reads it: the expression and the type run to the next `,`, markup included.
    pub(super) fn icu_start(&mut self) -> PResult<()> {
        let start = self.pos;
        let len = self.bytes.len();
        let invalid = || error(DiagnosticKind::InvalidIcu, span(start, start + 1));
        let Some(comma) = self.find(start + 1, len, b",") else { return invalid() };
        let type_start = self.skip_ws(comma + 1);
        let Some(type_end) = self.find(type_start, len, b",") else { return invalid() };
        let icu = Icu {
            expression: self.trimmed(start + 1, comma),
            type_name: self.trimmed(type_start, type_end),
        };
        let id = self
            .push(NodeKind::Icu(ArenaBox::new_in(icu, &self.allocator)), span(start, type_end + 1));
        self.open_container(id, true);
        self.pos = type_end + 1;
        self.icu_next_case()
    }

    /// After an ICU expression's head or one of its cases: the next case up to its `{`, or the `}` closing the expression.
    /// The whitespace before either is skipped, as Angular's lexer does.
    fn icu_next_case(&mut self) -> PResult<()> {
        let at = self.skip_ws(self.pos);
        if self.byte(at) == Some(b'}') {
            self.close_container(at);
            return Ok(());
        }
        let Some(open) = self.find(at, self.bytes.len(), b"{") else {
            return error(DiagnosticKind::InvalidIcu, span(at, at + 1));
        };
        let case = IcuCase { value: self.trimmed(at, open), open: span(open, open + 1).start };
        let id = self.push(NodeKind::IcuCase(case), span(at, open + 1));
        self.open_container(id, true);
        self.pos = open + 1;
        Ok(())
    }

    /// Closes the container on top of the stack at the `}` at `at`.
    fn close_container(&mut self, at: usize) {
        let open = self.stack.pop().expect("a container is open");
        let node = &mut self.nodes[open.id.index()];
        node.span = span(node.span.start as usize, at + 1);
        if let NodeKind::Block(block) = &mut node.kind {
            block.close = Some(span(at, at + 1));
        }
        self.pos = at + 1;
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

    /// `@let name = value;`; `pos` is at `@`.
    /// As Angular's lexer reads it: the value runs to the first `;` outside string literals, markup included.
    pub(super) fn let_declaration(&mut self) -> PResult<()> {
        let start = self.pos;
        let incomplete = |end| error(DiagnosticKind::IncompleteLet, span(start, end));
        let name_start = self.skip_ws(start + 4);
        if name_start == start + 4 {
            return incomplete(name_start);
        }
        let name_end = if self.byte(name_start).is_some_and(|b| b.is_ascii_digit()) {
            name_start
        } else {
            self.skip(name_start, |b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_'))
        };
        let eq = self.skip_ws(name_end);
        if self.byte(eq) != Some(b'=') {
            return incomplete(eq);
        }
        let value_start = self.skip_ws(eq + 1);
        // Unlike in an interpolation, a `\` escapes inside a string only
        let mut quote = None;
        let mut at = value_start;
        let semicolon = loop {
            let Some(b) = self.byte(at) else { return incomplete(at.min(self.bytes.len())) };
            match b {
                b';' if quote.is_none() => break at,
                b'\\' if quote.is_none() => at += 1,
                _ => at += string_step(b, &mut quote, true).0,
            }
        };
        let value_end = value_start + self.bytes[value_start..semicolon].trim_ascii_end().len();
        let declaration = LetDeclaration {
            name: span(name_start, name_end),
            value: span(value_start, value_end),
        };
        self.pos = semicolon + 1;
        self.push(
            NodeKind::LetDeclaration(ArenaBox::new_in(declaration, &self.allocator)),
            span(start, self.pos),
        );
        Ok(())
    }

    /// `}`: closes the innermost block, and the elements in it whose end tag may be omitted;
    /// in an ICU case, the case and every element left open in it, as Angular builds a case's content on its own,
    /// then reads the next case.
    pub(super) fn block_close(&mut self) -> PResult<()> {
        let at = self.pos;
        let icu = self.context().icu;
        // None in a conditional comment's content, parsed apart
        let index = self.stack.iter().rposition(|open| open.block);
        let closed = index.is_some_and(|index| {
            if icu {
                self.close_down_to(index + 1, at);
                true
            } else {
                self.close_omitted_above(index + 1, at)
            }
        });
        if !closed {
            return error(DiagnosticKind::UnexpectedBlockClose, span(at, at + 1));
        }
        self.close_container(at);
        if icu { self.icu_next_case() } else { Ok(()) }
    }
}

/// `name`'s kind, as Angular's compiler reads it (see [`AngularAttribute`]).
/// Its keyword prefixes (`#`, `let-`, …) start with none of the delimiters, so they fall through to `None`.
pub(super) fn attribute(name: &str) -> Option<AngularAttribute> {
    let (kind, close) = match name.as_bytes().first()? {
        b'*' => return Some(AngularAttribute::Template),
        b'[' if name.starts_with("[(") => (AngularAttribute::TwoWay, ")]"),
        b'[' => (AngularAttribute::Property, "]"),
        b'(' => (AngularAttribute::Event, ")"),
        _ => {
            return (name == "i18n" || name.starts_with("i18n-")).then_some(AngularAttribute::I18n);
        }
    };
    (name.ends_with(close) && name.len() > close.len() * 2).then_some(kind)
}

/// The `}}` closing an interpolation whose expression starts at `from`, outside string literals;
/// after a `//` comment, quotes no longer count.
/// `Err` with where the search stopped when a tag starts first, even in a string, or `limit` comes:
/// Angular reads what it searched as text.
/// As in Angular's lexer, the character right after `//` is read without looking for a tag (`{{ a //<b }}` holds no tag).
pub(super) fn interpolation_end(bytes: &[u8], from: usize, limit: usize) -> Result<usize, usize> {
    let (mut quote, mut comment) = (None, false);
    let mut at = from;
    while at < limit {
        if is_tag_start(bytes, at) {
            return Err(at);
        }
        if quote.is_none() {
            match &bytes[at..limit] {
                [b'}', b'}', ..] => return Ok(at),
                [b'/', b'/', ..] => {
                    comment = true;
                    at += 2;
                }
                _ => {}
            }
        }
        let Some(&b) = bytes[..limit].get(at) else { break };
        at += string_step(b, &mut quote, !comment).0;
    }
    Err(at.min(limit))
}

/// The expressions of the `{{ }}` in `[from, limit)`, as Angular reads them in text and attribute values.
pub(super) fn interpolations(
    bytes: &[u8],
    from: usize,
    limit: usize,
) -> impl Iterator<Item = Span> + '_ {
    let mut at = from;
    std::iter::from_fn(move || {
        while let Some(open) = memchr::memmem::find(&bytes[at..limit], b"{{").map(|i| at + i) {
            match interpolation_end(bytes, open + 2, limit) {
                Ok(close) => {
                    at = close + 2;
                    return Some(span(open + 2, close));
                }
                Err(stop) => at = stop.max(open + 2),
            }
        }
        None
    })
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
