//! Angular's expressions, on its compiler's lexer's tokens, which takes no JS parse:
//! the lexer, shared with the microsyntax (`microsyntax.rs`),
//! an expression's pipes at the root and its comment (`parsePipe`, `_stripComments`),
//! and an action's statements and its comment (`parseAction`).

use oxc_allocator::{Allocator, ArenaBox, ArenaVec};

use super::angular::string_step;
use crate::{
    ast::{AngularAction, AngularCode, AngularExpression, NestedPipe, NestedPipeParent, Pipe},
    span::{Span, span},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Identifier,
    /// Angular's 14 (`let`, `as`, `in`, `typeof`, …).
    Keyword,
    /// A string, number, regular expression or template literal: a value.
    Literal,
    /// `(` `)` `[` `]` `{` `}` `,` `:` `;` `.`.
    Character(u8),
    Operator,
}

#[derive(Clone, Copy)]
pub(super) struct Token<'s> {
    pub(super) kind: Kind,
    pub(super) text: &'s str,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Token<'_> {
    pub(super) fn is(&self, kind: Kind, text: &str) -> bool {
        self.kind == kind && self.text == text
    }

    pub(super) fn is_char(&self, c: u8) -> bool {
        self.kind == Kind::Character(c)
    }

    /// A prefix operator, where an operand is expected (`!a`, `-a`, `typeof a`).
    pub(super) fn is_prefix(&self) -> bool {
        matches!(self.kind, Kind::Operator if matches!(self.text, "!" | "-" | "+"))
            || self.is(Kind::Keyword, "typeof")
            || self.is(Kind::Keyword, "void")
    }

    /// A string literal, its quote a reference's or not (`&quot;x&quot;`).
    pub(super) fn is_string(&self) -> bool {
        self.kind == Kind::Literal
            && unit(self.text.as_bytes(), 0, true).is_some_and(|(b, _)| matches!(b, b'\'' | b'"'))
    }
}

const KEYWORDS: [&str; 14] = [
    "var",
    "let",
    "as",
    "null",
    "undefined",
    "true",
    "false",
    "if",
    "else",
    "this",
    "typeof",
    "void",
    "in",
    "instanceof",
];

/// An expression's tokens, before the comment it ends in, which Angular strips first (`_stripComments`).
struct Code<'s> {
    source: &'s str,
    /// Its characters are read decoded ([`Lexer::decoded`]).
    decoded: bool,
    tokens: Vec<Token<'s>>,
    comment: Option<Span>,
    /// Where the tokens' offsets start in the source.
    base: usize,
}

impl<'s> Code<'s> {
    /// `None` where Angular reports an error: a character it does not lex, `{{` and `}}` in the code (`_checkNoInterpolation`),
    /// a `,` before a `)`, which a call's arguments and an arrow's params do not end in (`f(a,)`; `[a,]` and `{a: 1,}` do).
    fn read(source: &'s str, at: Span, decoded: bool) -> Option<Self> {
        let s = at.slice(source);
        let base = at.start as usize;
        // Most code holds no reference: its characters are its bytes
        let decoded = decoded && memchr::memchr(b'&', s.as_bytes()).is_some();
        let comment_at = comment_start(s, decoded);
        let comment = comment_at
            .map(|start| span(base + start, base + s.trim_end_matches(is_whitespace).len()));
        let code = &s[..comment_at.unwrap_or(s.len())];
        let mut tokens = vec![];
        Lexer::new(code, decoded).scan(&mut tokens, false)?;
        if tokens.windows(2).any(|w| w[0].is_char(b',') && w[1].is_char(b')')) {
            return None;
        }
        if code.contains("{{") {
            let adjacent = |c: u8| {
                tokens
                    .windows(2)
                    .position(|w| w[0].is_char(c) && w[1].is_char(c) && w[0].end == w[1].start)
            };
            if let (Some(open), Some(close)) = (adjacent(b'{'), adjacent(b'}'))
                && open < close
            {
                return None;
            }
        }
        Some(Self { source, decoded, tokens, comment, base })
    }

    /// Where the tokens in `[first, end)` are; `None` when there are none.
    fn span(&self, first: usize, end: usize) -> Option<Span> {
        (first < end).then(|| {
            span(self.base + self.tokens[first].start, self.base + self.tokens[end - 1].end)
        })
    }

    /// The tokens in `[first, end)` as an action's statement, which no pipe follows; `None` when there are none.
    fn statement<'a>(
        &self,
        allocator: &'a Allocator,
        first: usize,
        end: usize,
    ) -> Option<AngularCode<'a>> {
        Some(AngularCode {
            span: self.span(first, end)?,
            conditional: false,
            nested: ArenaVec::new_in(&allocator),
        })
    }

    /// The tokens in `[first, end)` as a piece of code; `None` when there are none, or where Angular reports an error in a nested pipe.
    fn piece<'a>(
        &self,
        allocator: &'a Allocator,
        first: usize,
        end: usize,
    ) -> Option<AngularCode<'a>> {
        let tokens = &self.tokens[first..end];
        Some(AngularCode {
            span: self.span(first, end)?,
            conditional: tokens.iter().any(|t| t.is(Kind::Operator, "?")) && is_conditional(tokens),
            nested: self.nested(allocator, first, end)?,
        })
    }

    /// The outermost expressions with pipes in the tokens in `[first, end)` ([`AngularCode::nested`]):
    /// an element of a bracket with a `|` at its own level and no `?`, read as an expression of its own (`parsePipe`), the pipes in it its own;
    /// `None` where Angular reports an error in one.
    fn nested<'a>(
        &self,
        allocator: &'a Allocator,
        first: usize,
        end: usize,
    ) -> Option<ArenaVec<'a, NestedPipe<'a>>> {
        let mut nested = ArenaVec::new_in(&allocator);
        let tokens = &self.tokens;
        // No pipe below the root: nothing to read, as in most code
        if !tokens[first..end].iter().any(|t| t.is(Kind::Operator, "|")) {
            return Some(nested);
        }
        // The outermost ones found so far, read once the scan ends: an inner one is read with the outer
        let mut found: Vec<Found> = vec![];
        let mut stack: Vec<Element> = vec![];
        // The `?`s at the root whose `:` is yet to come, and the last conditional `:` seen
        let mut root_questions = 0u32;
        let mut conditional_colon: Option<usize> = None;
        for i in first..end {
            let t = tokens[i];
            match t.kind {
                Kind::Character(c @ (b'(' | b'[' | b'{')) => {
                    // A call's arguments and an index follow a value (`a(b)`, `a!(b)`, `a?.(b)`); an array and parens around an expression do not
                    let after_value = self.follows_value(i, first)
                        || (i > first && tokens[i - 1].is(Kind::Operator, "?."));
                    let call = c == b'(' && after_value;
                    // Parens right after a conditional's `?` or `:` are one of its branches
                    let in_conditional = i > first
                        && (tokens[i - 1].is(Kind::Operator, "?")
                            || conditional_colon == Some(i - 1));
                    let mut element =
                        Element::new(i, i, c == b'(' && !after_value, c == b'{', call);
                    element.in_conditional = in_conditional;
                    stack.push(element);
                }
                Kind::Character(b')' | b']' | b'}') => {
                    let element = stack.pop()?;
                    // The element its parens are all of: of a call's arguments, an index or an array, which Angular reads the pipe without them,
                    // or an object's value (whose parens Prettier prints) or a spread's (`...` before them), whose stay as written
                    let all_of = stack.last().filter(|outer| {
                        element.group
                            && !outer.group
                            && outer.start == Some(element.open)
                            && i + 1 < end
                            && matches!(
                                tokens[i + 1].kind,
                                Kind::Character(b',' | b')' | b']' | b'}')
                            )
                    });
                    let kept =
                        all_of.is_some_and(|outer| outer.object || self.after_spread(element.open));
                    // An object's last value (its `}` checked, as brackets may not match: `{a: b)`), or parens all of one
                    let last_value = (element.object && tokens[i].is_char(b'}'))
                        || (kept && self.ends_object(i + 1));
                    // Only for parens `close` keeps: a pipe in them, not a conditional's branch nor all of an element
                    let parent = if element.group
                        && element.pipe
                        && !element.conditional
                        && (all_of.is_none() || kept)
                    {
                        let object_value = all_of.is_some_and(|outer| outer.object)
                            && !self.after_spread(element.open);
                        self.parent(&element, i, stack.last(), object_value, first, end)
                    } else {
                        NestedPipeParent::Other
                    };
                    self.close(
                        &element,
                        i,
                        all_of.is_some() && !kept,
                        last_value,
                        parent,
                        &mut found,
                    )?;
                }
                Kind::Character(b',') => {
                    if let Some(element) = stack.last_mut() {
                        let last_value = element.object && self.ends_object(i);
                        self.close(
                            element,
                            i,
                            false,
                            last_value,
                            NestedPipeParent::Other,
                            &mut found,
                        )?;
                        element.next(i);
                    }
                }
                Kind::Character(b':') => {
                    let questions = match stack.last_mut() {
                        Some(element) if element.object && element.start.is_none() => {
                            element.start = Some(i + 1);
                            None
                        }
                        level => Some(level.map_or(&mut root_questions, |e| &mut e.questions)),
                    };
                    // A conditional's `:`, which ends its `?`'s branch
                    if let Some(questions) = questions
                        && *questions > 0
                    {
                        *questions -= 1;
                        conditional_colon = Some(i);
                    }
                }
                Kind::Operator if t.text == "|" => {
                    if let Some(element) = stack.last_mut() {
                        element.pipe = true;
                    }
                }
                // A spread's expression is the element (`parseSpreadElement`), an object's too, which has no `:`
                Kind::Operator if t.text == "..." => {
                    if let Some(element) = stack.last_mut()
                        && !element.group
                        && matches!(tokens[i - 1].kind, Kind::Character(b'(' | b'[' | b'{' | b','))
                    {
                        element.start = Some(i + 1);
                    }
                }
                // Before a pipe, a conditional takes it in a branch (`a ? b : c | p`);
                // after one, it is the pipe's argument, as at the root (`expression`)
                Kind::Operator if t.text == "?" => {
                    if let Some(element) = stack.last_mut()
                        && !element.pipe
                    {
                        element.conditional = true;
                    }
                    *stack.last_mut().map_or(&mut root_questions, |e| &mut e.questions) += 1;
                }
                _ => {}
            }
        }
        for Found { span, parens, last_value, spread, inner } in found {
            let expression = expression(allocator, self.source, inner, self.decoded)?;
            nested.push(NestedPipe { span, parens, last_value, spread, expression });
        }
        Some(nested)
    }

    /// What the parens of `element`, closed at `close`, are part of ([`NestedPipe::parens`]), in the tokens in `[first, end)`:
    /// what follows them binds first (`!(a | p).b` is a member's object), then what precedes them.
    /// `level`: the element they are in, `None` at the root; `object_value`: they are all of an object's value.
    fn parent(
        &self,
        element: &Element,
        close: usize,
        level: Option<&Element>,
        object_value: bool,
        first: usize,
        end: usize,
    ) -> NestedPipeParent {
        let t = |at: usize| (first..end).contains(&at).then(|| self.tokens[at]);
        let call_argument = level.is_some_and(|level| level.call);
        let after = t(close + 1);
        let optional = after.is_some_and(|a| a.is(Kind::Operator, "?."));
        let next = if optional { t(close + 2) } else { after };
        match next {
            Some(n) if n.is_char(b'(') => return NestedPipeParent::Callee,
            Some(n) if n.is_char(b'[') => return NestedPipeParent::MemberObject { computed: true },
            Some(n) if n.is_char(b'.') || (optional && n.kind == Kind::Identifier) => {
                return NestedPipeParent::MemberObject { computed: false };
            }
            _ => {}
        }
        // All of an operand: nothing binds them on either side (`c ? (a | p) : d`, not `c ? (a | p) + 1 : d`)
        let before = (element.open > first).then(|| self.tokens[element.open - 1]);
        let starts = before.is_none_or(|b| {
            matches!(b.kind, Kind::Character(b'(' | b'[' | b'{' | b',' | b':'))
                || b.is(Kind::Operator, "?")
        });
        let ends = after.is_none_or(|a| {
            matches!(a.kind, Kind::Character(b')' | b']' | b'}' | b',' | b':' | b';'))
                || a.is(Kind::Operator, "|")
        });
        // A prefix one: no value before it (`a - (b | p)` is a binary's)
        if before.is_some_and(|b| b.is_prefix() && !self.follows_value(element.open - 1, first)) {
            NestedPipeParent::UnaryArgument
        } else if (element.in_conditional && ends)
            || (starts && after.is_some_and(|a| a.is(Kind::Operator, "?")))
        {
            NestedPipeParent::Conditional { call_argument }
        } else if object_value {
            NestedPipeParent::ObjectValue
        } else {
            NestedPipeParent::Other
        }
    }

    /// The token `at`, in the tokens from `first`, follows a value: what a call's arguments, an index or a postfix `!` follow,
    /// a non-null one too (`a!`).
    fn follows_value(&self, at: usize, first: usize) -> bool {
        if at <= first {
            return false;
        }
        let t = self.tokens[at - 1];
        matches!(t.kind, Kind::Identifier | Kind::Literal)
            || t.is_char(b')')
            || t.is_char(b']')
            || (t.kind == Kind::Keyword
                && matches!(t.text, "this" | "null" | "undefined" | "true" | "false"))
            || (t.is(Kind::Operator, "!") && self.follows_value(at - 1, first))
    }

    /// The token `at` follows a spread's `...`: it starts the spread's expression.
    fn after_spread(&self, at: usize) -> bool {
        at > 0 && self.tokens[at - 1].is(Kind::Operator, "...")
    }

    /// The token `at` ends an object's last value: its `}`, or a trailing `,` before it, which a flat object drops.
    fn ends_object(&self, at: usize) -> bool {
        let t = |at: usize| self.tokens.get(at);
        t(at).is_some_and(|t| t.is_char(b'}'))
            || (t(at).is_some_and(|t| t.is_char(b','))
                && t(at + 1).is_some_and(|t| t.is_char(b'}')))
    }

    /// Ends `element` at the token `end` (its `,` or closing bracket): one with a pipe goes in `found`, the ones inside it out.
    /// `alone`: its parens are all of the element they are in, which Angular reads the pipe without too.
    fn close(
        &self,
        element: &Element,
        end: usize,
        alone: bool,
        last_value: bool,
        parent: NestedPipeParent,
        found: &mut Vec<Found>,
    ) -> Option<()> {
        let Some(start) = element.start.filter(|_| element.pipe && !element.conditional) else {
            return Some(());
        };
        let inner = self.span(start, end)?;
        // Parens Angular reads it without too stay in the code, which JS drops around a name
        let in_parens = element.group && !alone;
        let span = if in_parens { self.span(element.open, end + 1)? } else { inner };
        let spread = self.after_spread(if in_parens { element.open } else { start });
        while found.last().is_some_and(|f| f.span.start >= span.start) {
            found.pop();
        }
        let parens = in_parens.then_some(parent);
        found.push(Found { span, parens, last_value, spread, inner });
        Some(())
    }
}

/// An outermost nested pipe found, read after the scan ([`Code::nested`]).
struct Found {
    span: Span,
    parens: Option<NestedPipeParent>,
    last_value: bool,
    spread: bool,
    /// The expression's
    inner: Span,
}

/// A bracket's current element, from `start` ([`Code::nested`]).
struct Element {
    open: usize,
    /// Parens around an expression (not a call's arguments): its one element is replaced with them
    group: bool,
    /// `{`: an element starts at a key's `:`, its value
    object: bool,
    start: Option<usize>,
    pipe: bool,
    conditional: bool,
    /// `(` after a value: a call's arguments
    call: bool,
    /// The `?`s in it whose `:` is yet to come
    questions: u32,
    /// It opens right after a conditional's `?` or `:`
    in_conditional: bool,
}

impl Element {
    /// From the bracket `open`, its first element past `from` (an object's starts at a key's `:`).
    fn new(open: usize, from: usize, group: bool, object: bool, call: bool) -> Self {
        Self {
            open,
            group,
            object,
            start: (!object).then_some(from + 1),
            pipe: false,
            conditional: false,
            call,
            questions: 0,
            in_conditional: false,
        }
    }

    /// The next one, after the `,` at `at`.
    fn next(&mut self, at: usize) {
        *self = Self::new(self.open, at, self.group, self.object, self.call);
    }
}

/// The action at `at` (an event binding's value) as Angular reads it (`parseAction`): its statements, between the root's `;`;
/// `None` where it reports an error the tokens show (no statement and no comment, a pipe, a bracket left open, `{{` and `}}` in it).
pub fn action<'a>(
    allocator: &'a Allocator,
    source: &str,
    at: Span,
) -> Option<ArenaBox<'a, AngularAction<'a>>> {
    let code = Code::read(source, at, true)?;
    let mut statements = ArenaVec::new_in(&allocator);
    let mut depth = 0usize;
    let mut first = 0;
    for (i, t) in code.tokens.iter().enumerate() {
        match t.kind {
            Kind::Character(b'(' | b'[' | b'{') => depth += 1,
            Kind::Character(b')' | b']' | b'}') => depth = depth.checked_sub(1)?,
            // `Cannot have a pipe in an action expression`, anywhere in it
            Kind::Operator if t.text == "|" => return None,
            Kind::Character(b';') if depth == 0 => {
                statements.extend(code.statement(allocator, first, i));
                first = i + 1;
            }
            _ => {}
        }
    }
    if depth > 0 {
        return None;
    }
    statements.extend(code.statement(allocator, first, code.tokens.len()));
    if statements.is_empty() && code.comment.is_none() {
        return None;
    }
    Some(ArenaBox::new_in(AngularAction { statements, comment: code.comment }, &allocator))
}

/// The expression at `at` (`{{ }}`'s, a property binding's value, a microsyntax's, a block parameter's, `@let`'s)
/// as Angular reads it (`parseBinding`): its code, the pipes at its root and its comment;
/// `decoded`: in an attribute's value or a text, whose character references Angular decodes first (not in a block's parameters nor `@let`'s value);
/// `None` where it reports an error the tokens show (no code, a `;`, a `:` no `?` or pipe takes, a pipe's name missing,
/// a bracket left open, `{{` and `}}` in it).
pub fn expression<'a>(
    allocator: &'a Allocator,
    source: &str,
    at: Span,
    decoded: bool,
) -> Option<ArenaBox<'a, AngularExpression<'a>>> {
    let read = Code::read(source, at, decoded)?;
    let tokens = &read.tokens;

    // The root's `|` and the `:` after one, which end a segment: the code, a pipe's name or one of its arguments.
    // Angular has no bitwise or: any other `|` is a pipe too, which JS does not read so
    let mut breaks = vec![];
    let mut depth = 0usize;
    // The root's `?` waiting for their `:`; whether the segment has one (its conditional takes the pipes after it)
    let (mut conditionals, mut conditional) = (0usize, false);
    for (i, t) in tokens.iter().enumerate() {
        match t.kind {
            Kind::Character(b'(' | b'[' | b'{') => depth += 1,
            Kind::Character(b')' | b']' | b'}') => depth = depth.checked_sub(1)?,
            _ if depth > 0 => {}
            Kind::Character(b';') => return None,
            // Its branch takes the pipes after it (as in an element, `Code::nested`)
            Kind::Operator if t.text == "?" => {
                conditionals += 1;
                conditional = true;
            }
            Kind::Character(b':') if conditionals > 0 => conditionals -= 1,
            Kind::Character(b':') if breaks.is_empty() => return None,
            Kind::Character(b':') => breaks.push(i),
            Kind::Operator if t.text == "|" && !conditional => breaks.push(i),
            _ => {}
        }
    }
    if depth > 0 {
        return None;
    }
    let mut ends = breaks.iter().copied().chain([tokens.len()]);
    let code_end = ends.next()?;
    let code = read.piece(allocator, 0, code_end)?;
    let mut pipes: ArenaVec<'a, Pipe<'a>> = ArenaVec::new_in(&allocator);
    let mut first = code_end + 1;
    for end in ends {
        if tokens[first - 1].is(Kind::Operator, "|") {
            // A name is one token
            if end - first != 1 || !matches!(tokens[first].kind, Kind::Identifier | Kind::Keyword) {
                return None;
            }
            pipes.push(Pipe {
                name: read.span(first, end)?,
                arguments: ArenaVec::new_in(&allocator),
            });
        } else {
            pipes.last_mut()?.arguments.push(read.piece(allocator, first, end)?);
        }
        first = end + 1;
    }
    Some(ArenaBox::new_in(AngularExpression { code, pipes, comment: read.comment }, &allocator))
}

/// Whether the root of `tokens` (a piece's), past parens around all of them, is a conditional:
/// a `?` at the root, but after an arrow's `=>` or an assignment's operator, which take it (`x => a ? b : c`).
fn is_conditional(mut tokens: &[Token<'_>]) -> bool {
    while let [first, inner @ .., last] = tokens
        && first.is_char(b'(')
        && last.is_char(b')')
        && encloses(tokens)
    {
        tokens = inner;
    }
    let mut depth = 0usize;
    for t in tokens {
        match t.kind {
            Kind::Character(b'(' | b'[' | b'{') => depth += 1,
            Kind::Character(b')' | b']' | b'}') => depth = depth.saturating_sub(1),
            Kind::Operator if depth == 0 && t.text == "?" => return true,
            Kind::Operator if depth == 0 && takes_conditional(t.text) => return false,
            _ => {}
        }
    }
    false
}

/// An operator whose right side is an expression a conditional at the root is in (`=>`, `=`, `+=`, …).
fn takes_conditional(operator: &str) -> bool {
    operator == "=>"
        || (operator.ends_with('=')
            && !matches!(operator, "==" | "===" | "!=" | "!==" | "<=" | ">="))
}

/// The bracket `tokens` open with closes at their last token (`(a) + (b)` is two).
fn encloses(tokens: &[Token<'_>]) -> bool {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate() {
        match t.kind {
            Kind::Character(b'(' | b'[' | b'{') => depth += 1,
            Kind::Character(b')' | b']' | b'}') => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == tokens.len();
                }
            }
            _ => {}
        }
    }
    false
}

/// Where a comment starts in `s` (Angular's `_commentStart`): the first `//` outside quotes, `` ` `` one too,
/// in the characters it reads ([`unit()`]).
fn comment_start(s: &str, decoded: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    // Most code holds no `/`, which a reference may spell only (`&sol;`)
    if !decoded && memchr::memchr(b'/', bytes).is_none() {
        return None;
    }
    let mut quote = None;
    let mut at = 0;
    while let Some((char, len)) = unit(bytes, at, decoded) {
        let next = unit(bytes, at + len, decoded);
        if char == b'/' && quote.is_none() && next.is_some_and(|(next, _)| next == b'/') {
            return Some(at);
        }
        let (step, _) = string_step(char, &mut quote, true);
        at += len;
        // A `\` takes the next character
        if step == 2 {
            at += next.map_or(0, |(_, len)| len);
        }
    }
    None
}

/// Angular's whitespace: every control character, the space, and NBSP.
fn is_whitespace(c: char) -> bool {
    c <= ' ' || c == '\u{a0}'
}

pub(super) struct Lexer<'s> {
    s: &'s str,
    at: usize,
    /// It reads a character reference as its character ([`unit()`]): in an attribute's value or a text, which Angular decodes first.
    decoded: bool,
}

impl<'s> Lexer<'s> {
    pub(super) fn new(s: &'s str, decoded: bool) -> Self {
        // Most code holds no reference: its characters are its bytes
        Self { s, at: 0, decoded: decoded && memchr::memchr(b'&', s.as_bytes()).is_some() }
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.s.as_bytes().get(at).copied()
    }

    /// Tokens up to the end, or, `in_template`, up to the `}` closing a template literal's `${`, which it consumes.
    pub(super) fn scan(&mut self, tokens: &mut Vec<Token<'s>>, in_template: bool) -> Option<()> {
        let first = tokens.len();
        let mut braces = 0;
        loop {
            // Angular's whitespace: every control character, the space, and NBSP
            while let Some((b, len)) = self.unit(self.at) {
                match b {
                    ..=b' ' => self.at += len,
                    0xC2 if self.byte(self.at + 1) == Some(0xA0) => self.at += 2,
                    _ => break,
                }
            }
            let start = self.at;
            let Some((b, len)) = self.unit(start) else { return (!in_template).then_some(()) };
            // An operator's text is the operator, which references may spell (`&amp;&amp;` is `&&`)
            let mut operator = None;
            let kind = match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                    self.at =
                        self.skip(start, |b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'));
                    if KEYWORDS.contains(&&self.s[start..self.at]) {
                        Kind::Keyword
                    } else {
                        Kind::Identifier
                    }
                }
                b'0'..=b'9' => {
                    self.number();
                    Kind::Literal
                }
                b'.' if self.unit(start + len).is_some_and(|(b, _)| b.is_ascii_digit()) => {
                    self.number();
                    Kind::Literal
                }
                b'.' if let Some(end) = self.spells(start, "...") => {
                    self.at = end;
                    operator = Some("...");
                    Kind::Operator
                }
                b'\'' | b'"' => {
                    self.at += len;
                    self.string(b)?;
                    Kind::Literal
                }
                b'`' => {
                    self.at += len;
                    self.template(tokens)?;
                    Kind::Literal
                }
                b'{' => {
                    braces += 1;
                    self.at += len;
                    Kind::Character(b)
                }
                b'}' if in_template && braces == 0 => {
                    self.at += len;
                    return Some(());
                }
                b'}' => {
                    braces -= 1;
                    self.at += len;
                    Kind::Character(b)
                }
                b'(' | b')' | b'[' | b']' | b',' | b':' | b';' | b'.' => {
                    self.at += len;
                    Kind::Character(b)
                }
                b'/' if starts_regex(&tokens[first..]) => {
                    self.regex()?;
                    Kind::Literal
                }
                _ => {
                    let (text, end) = if self.decoded {
                        OPERATORS.into_iter().find_map(|op| Some((op, self.spells(start, op)?)))?
                    } else {
                        let rest = &self.s.as_bytes()[start..];
                        let op =
                            OPERATORS.into_iter().find(|op| rest.starts_with(op.as_bytes()))?;
                        (op, start + op.len())
                    };
                    self.at = end;
                    operator = Some(text);
                    Kind::Operator
                }
            };
            let text = operator.unwrap_or(&self.s[start..self.at]);
            tokens.push(Token { kind, text, start, end: self.at });
        }
    }

    #[inline]
    fn unit(&self, at: usize) -> Option<(u8, usize)> {
        unit(self.s.as_bytes(), at, self.decoded)
    }

    /// Past the characters from `from` that `pred` takes.
    fn skip(&self, mut at: usize, pred: impl Fn(u8) -> bool) -> usize {
        if !self.decoded {
            return at + self.s.as_bytes()[at..].iter().take_while(|&&b| pred(b)).count();
        }
        while let Some((b, len)) = self.unit(at)
            && pred(b)
        {
            at += len;
        }
        at
    }

    /// Past `text`, when the characters from `from` spell it.
    fn spells(&self, from: usize, text: &str) -> Option<usize> {
        text.bytes().try_fold(from, |at, expected| {
            self.unit(at).filter(|&(b, _)| b == expected).map(|(_, len)| at + len)
        })
    }

    /// Digits with `_`, a fraction and an exponent; Angular's errors in them are not repeated.
    fn number(&mut self) {
        self.at = self.skip(self.at, |b| b.is_ascii_digit() || matches!(b, b'_' | b'.'));
        if let Some((b'e' | b'E', len)) = self.unit(self.at) {
            self.at += len;
            if let Some((b'+' | b'-', len)) = self.unit(self.at) {
                self.at += len;
            }
            self.at = self.skip(self.at, |b| b.is_ascii_digit());
        }
    }

    /// After its opening quote: through the closing one.
    fn string(&mut self, quote: u8) -> Option<()> {
        loop {
            let (b, len) = self.unit(self.at)?;
            self.at += len;
            match b {
                b'\\' => self.at += self.unit(self.at)?.1,
                b if b == quote => return Some(()),
                _ => {}
            }
        }
    }

    /// After a `` ` ``: through the closing one, its `${ }` scanned as tokens of their own (not kept).
    fn template(&mut self, tokens: &mut Vec<Token<'s>>) -> Option<()> {
        loop {
            let (b, len) = self.unit(self.at)?;
            self.at += len;
            match b {
                b'\\' => self.at += self.unit(self.at)?.1,
                b'`' => return Some(()),
                b'$' if let Some((b'{', len)) = self.unit(self.at) => {
                    self.at += len;
                    let len = tokens.len();
                    self.scan(tokens, true)?;
                    tokens.truncate(len);
                }
                _ => {}
            }
        }
    }

    fn regex(&mut self) -> Option<()> {
        self.at += self.unit(self.at)?.1;
        let mut class = false;
        loop {
            let (b, len) = self.unit(self.at)?;
            self.at += len;
            match b {
                b'\\' => self.at += self.unit(self.at)?.1,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => break,
                _ => {}
            }
        }
        self.at = self.skip(self.at, |b| b.is_ascii_alphabetic());
        Some(())
    }
}

/// Angular's `isStartOfRegex`: a `/` after nothing, an operator, `(`, `[`, `,` or `:`,
/// and after a `!` that negates (no value before it).
fn starts_regex(tokens: &[Token<'_>]) -> bool {
    let Some((last, before)) = tokens.split_last() else { return true };
    if last.is(Kind::Operator, "!") {
        return before
            .last()
            .is_none_or(|t| t.kind != Kind::Identifier && !t.is_char(b')') && !t.is_char(b']'));
    }
    last.kind == Kind::Operator || b"([,:".iter().any(|&c| last.is_char(c))
}

/// The operators Angular's lexer reads, longest first: any other character it reports.
const OPERATORS: [&str; 36] = [
    "===", "!==", "**=", "??=", "&&=", "||=", "==", "!=", "<=", ">=", "=>", "&&", "||", "??", "?.",
    "**", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "+", "-", "*", "/", "%", "<", ">", "=", "!",
    "?", "&", "|", "^",
];

/// The character at `at` of an expression's text and its length, as Angular reads it:
/// - `decoded` (an attribute's value, a text), a character reference as its character (`&quot;` is `"`):
///   an ASCII one, a space for NBSP, `0x80` for any other, which only a string holds (Angular reports one elsewhere)
/// - else, and any other byte, as written
#[inline]
fn unit(bytes: &[u8], at: usize, decoded: bool) -> Option<(u8, usize)> {
    let b = *bytes.get(at)?;
    if decoded
        && b == b'&'
        && let Some(reference) = character_reference(&bytes[at..])
    {
        return Some(reference);
    }
    Some((b, 1))
}

/// The character reference `bytes` starts with, a name or a number ended by `;` as Angular reads one: its character ([`unit()`]) and its length.
// Rare: kept out of the lexer's loops
#[cold]
#[inline(never)]
fn character_reference(bytes: &[u8]) -> Option<(u8, usize)> {
    let len = bytes[1..].iter().take_while(|&&b| b.is_ascii_alphanumeric() || b == b'#').count();
    if len == 0 || bytes.get(1 + len) != Some(&b';') {
        return None;
    }
    // ASCII, by the bytes it takes
    let name = std::str::from_utf8(&bytes[1..=len]).ok()?;
    let char = if let Some(number) = name.strip_prefix('#') {
        let (digits, radix) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (number, 10),
        };
        match u32::from_str_radix(digits, radix).ok()? {
            code @ 0..=0x7F => u8::try_from(code).ok()?,
            0xA0 => b' ',
            _ => 0x80,
        }
    } else {
        named_reference(name).unwrap_or(0x80)
    };
    Some((char, len + 2))
}

/// The ASCII character of a named reference, or a space for NBSP.
fn named_reference(name: &str) -> Option<u8> {
    Some(match name {
        "Tab" => b'\t',
        "NewLine" => b'\n',
        "nbsp" | "NonBreakingSpace" => b' ',
        "excl" => b'!',
        "quot" | "QUOT" => b'"',
        "num" => b'#',
        "dollar" => b'$',
        "percnt" => b'%',
        "amp" | "AMP" => b'&',
        "apos" => b'\'',
        "lpar" => b'(',
        "rpar" => b')',
        "ast" | "midast" => b'*',
        "plus" => b'+',
        "comma" => b',',
        "period" => b'.',
        "sol" => b'/',
        "colon" => b':',
        "semi" => b';',
        "lt" | "LT" => b'<',
        "equals" => b'=',
        "gt" | "GT" => b'>',
        "quest" => b'?',
        "commat" => b'@',
        "lsqb" | "lbrack" => b'[',
        "bsol" => b'\\',
        "rsqb" | "rbrack" => b']',
        "Hat" => b'^',
        "lowbar" | "UnderBar" => b'_',
        "grave" | "DiacriticalGrave" => b'`',
        "lcub" | "lbrace" => b'{',
        "verbar" | "vert" | "VerticalLine" => b'|',
        "rcub" | "rbrace" => b'}',
        _ => return None,
    })
}
