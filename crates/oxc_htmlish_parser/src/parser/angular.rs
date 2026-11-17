//! Angular: `{{ }}` closed outside string literals, control flow blocks (`@if (a) { … }`), `@let`,
//! ICU expressions (`{n, plural, =0 {none}}`), and binding and i18n attribute names (`[x]`, `i18n-x`).

use oxc_allocator::ArenaBox;

use super::*;

/// A block's kind from its name as written ([`BlockKind`]).
fn block_kind(name: &str) -> BlockKind {
    if name.contains(['\n', '\r']) {
        return BlockKind::Unknown;
    }
    let words = name.split(is_js_space).filter(|word| !word.is_empty());
    BlockKind::KNOWN
        .into_iter()
        .find(|kind| kind.name().is_some_and(|known| words.clone().eq(known.split(' '))))
        .unwrap_or(BlockKind::Unknown)
}

impl<'a> Builder<'a> {
    /// An Angular block starts at `at`: a block name's first word, by prefix (`@iffy` is a block named `iffy`).
    pub(super) fn at_block(&self, at: usize) -> bool {
        self.byte(at) == Some(b'@')
            && BlockKind::KNOWN.into_iter().filter_map(BlockKind::name).any(|name| {
                let first = name.split(' ').next().unwrap_or(name);
                self.bytes[at + 1..].starts_with(first.as_bytes())
            })
    }

    /// `@let` starts at `at`, even without the whitespace after it (`@letter` is an incomplete one).
    pub(super) fn at_let(&self, at: usize) -> bool {
        self.bytes[at..].starts_with(b"@let")
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
        let kind = block_kind(&self.source[name_start..name_end]);
        let mut parameters = ArenaVec::new_in(&self.allocator);
        let mut open_end = name_end;
        if self.byte(at) == Some(b'(') {
            at = self.block_parameters(at + 1, kind, &mut parameters)?;
            open_end = at;
            at = self.skip_ws(at);
        }
        let body = self.byte(at) == Some(b'{');
        // `@default never;` has no body;
        // cases fall through to the next one without one.
        let never = kind == BlockKind::DefaultNever && self.byte(at) == Some(b';');
        if body || never {
            open_end = at + 1;
        } else if !(matches!(kind, BlockKind::Case | BlockKind::Default) && self.at_block(at)) {
            return error(DiagnosticKind::IncompleteBlock, span(start, open_end));
        }
        self.pos = open_end;
        // Under `ngNonBindable` its markers are text, its content its parent's
        let text = self.context().non_bindable;
        let id = if text {
            self.stack.last().map_or(self.base_parent, |open| open.id)
        } else {
            let block = Block {
                name: span(name_start, name_end),
                kind,
                parameters,
                open: span(start, open_end),
                close: None,
            };
            self.push(
                NodeKind::Block(ArenaBox::new_in(block, &self.allocator)),
                span(start, open_end),
            )
        };
        if body {
            self.open_container(id, false, text);
        }
        Ok(())
    }

    /// Opens a block, an ICU expression or a case at `id`; `icu`: its content is a case's;
    /// `text`: a block read as text, `id` its parent.
    fn open_container(&mut self, id: NodeId, icu: bool, text: bool) {
        let context = Context { icu, ..self.context() };
        self.stack.push(Open {
            id,
            name: "",
            braced: true,
            text,
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
        self.open_container(id, true, false);
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
        self.open_container(id, true, false);
        self.pos = open + 1;
        Ok(())
    }

    /// Closes the container on top of the stack at the `}` at `at`.
    fn close_container(&mut self, at: usize) {
        let open = self.stack.pop().expect("a container is open");
        self.pos = at + 1;
        if open.text {
            return;
        }
        let node = &mut self.nodes[open.id.index()];
        node.span = span(node.span.start as usize, at + 1);
        if let NodeKind::Block(block) = &mut node.kind {
            block.close = Some(span(at, at + 1));
        }
    }

    /// The parameters after `(`, up to the `)`, which the returned offset follows, of a `block`.
    fn block_parameters(
        &self,
        from: usize,
        block: BlockKind,
        out: &mut ArenaVec<'a, BlockParameter<'a>>,
    ) -> PResult<usize> {
        let push = |start: usize, end: usize, out: &mut ArenaVec<'a, BlockParameter<'a>>| {
            let parameter = self.trimmed(start, end);
            if !parameter.is_empty() {
                let index = out.len();
                out.push(control_flow::parameter(
                    self.allocator,
                    self.source,
                    block,
                    index,
                    parameter,
                ));
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
            self.skip(name_start, is_identifier_part)
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
        self.pos = semicolon + 1;
        // Text under `ngNonBindable`
        if self.context().non_bindable {
            return Ok(());
        }
        let value = self.trimmed(value_start, semicolon);
        let declaration = LetDeclaration {
            name: span(name_start, name_end),
            value,
            angular: angular_expression::expression(self.allocator, self.source, value),
        };
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
        let index = self.stack.iter().rposition(|open| open.braced);
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

/// An attribute's kind, as Angular's compiler reads its name (`parseAttribute`) and value (see [`AngularAttribute`]);
/// `None` for a plain one.
pub(super) fn attribute<'a>(
    allocator: &'a Allocator,
    source: &str,
    name: Span,
    value: Option<Span>,
) -> Option<AngularAttribute<'a>> {
    /// The prefixes it reads before the delimiters (`BIND_NAME_REGEXP`):
    /// the bindings' ours when a name follows, the others none of our kinds.
    const KEYWORDS: [&str; 7] = ["bind-", "let-", "ref-", "#", "on-", "bindon-", "@"];
    use BindingForm::{Delimited, Prefixed};
    let n = name.slice(source);
    let prefixed = |prefix: &str| n.strip_prefix(prefix).is_some_and(|target| !target.is_empty());
    // The delimiters by the prefix alone (`[(a)-]` binds nothing), around a name
    let delimited = |close: &str| n.ends_with(close) && n.len() > close.len() * 2;
    // An empty name (`<a =b>`) binds a value's `{{ }}` too
    let binding = match n.as_bytes().first() {
        Some(b'*') => {
            let bindings = value.and_then(|v| microsyntax::template_bindings(allocator, source, v));
            return Some(AngularAttribute::Template(
                bindings.map(|b| ArenaBox::new_in(b, &allocator)),
            ));
        }
        _ if n == "i18n" || n.starts_with("i18n-") => return Some(AngularAttribute::I18n),
        Some(b'[') if n.starts_with("[(") => {
            delimited(")]").then_some(AngularAttribute::TwoWay(None, Delimited))
        }
        Some(b'[') => delimited("]").then_some(AngularAttribute::Property(None, Delimited)),
        Some(b'(') => delimited(")").then_some(AngularAttribute::Event(None, Delimited)),
        _ if prefixed("bind-") => Some(AngularAttribute::Property(None, Prefixed)),
        _ if prefixed("on-") => Some(AngularAttribute::Event(None, Prefixed)),
        _ if prefixed("bindon-") => Some(AngularAttribute::TwoWay(None, Prefixed)),
        _ => None,
    };
    if let Some(mut kind) = binding {
        match &mut kind {
            AngularAttribute::Property(expression, _) | AngularAttribute::TwoWay(expression, _) => {
                *expression =
                    value.and_then(|v| angular_expression::expression(allocator, source, v));
            }
            AngularAttribute::Event(action, _) => {
                *action = value.and_then(|v| angular_expression::action(allocator, source, v));
            }
            _ => {}
        }
        return Some(kind);
    }
    if KEYWORDS.iter().any(|keyword| n.starts_with(keyword)) {
        return None;
    }
    let value = value?;
    let mut expressions =
        interpolations(source.as_bytes(), value.start as usize, value.end as usize).peekable();
    expressions.peek()?;
    let expressions = ArenaVec::from_iter_in(
        expressions.map(|expression| ValueInterpolation {
            expression,
            angular: angular_expression::expression(allocator, source, expression),
        }),
        &allocator,
    );
    Some(AngularAttribute::Interpolation(ArenaBox::new_in(expressions, &allocator)))
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
        while let Some(open) = memchr::memchr(b'{', &bytes[at..limit]).map(|i| at + i) {
            if open + 1 == limit || bytes[open + 1] != b'{' {
                at = open + 1;
                continue;
            }
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
pub(super) fn string_step(b: u8, quote: &mut Option<u8>, opens: bool) -> (usize, bool) {
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
