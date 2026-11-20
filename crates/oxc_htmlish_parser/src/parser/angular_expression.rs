//! Angular's expressions, on its compiler's lexer's tokens, which takes no JS parse:
//! the lexer, shared with the microsyntax (`microsyntax.rs`),
//! an expression's pipes at the root and its comment (`parsePipe`, `_stripComments`),
//! and an action's statements and its comment (`parseAction`).

use oxc_allocator::{Allocator, ArenaBox, ArenaVec};

use super::angular::string_step;
use crate::{
    ast::{AngularAction, AngularCode, AngularExpression, NestedPipe, Pipe},
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
    tokens: Vec<Token<'s>>,
    comment: Option<Span>,
    /// Where the tokens' offsets start in the source.
    base: usize,
}

impl<'s> Code<'s> {
    /// `None` where Angular reports an error: a character it does not lex, `{{` and `}}` in the code (`_checkNoInterpolation`),
    /// a `,` before a `)`, which a call's arguments and an arrow's params do not end in (`f(a,)`; `[a,]` and `{a: 1,}` do).
    fn read(source: &'s str, at: Span) -> Option<Self> {
        let s = at.slice(source);
        let base = at.start as usize;
        let comment_at = comment_start(s);
        let comment = comment_at
            .map(|start| span(base + start, base + s.trim_end_matches(is_whitespace).len()));
        let code = &s[..comment_at.unwrap_or(s.len())];
        let mut tokens = vec![];
        Lexer::new(code).scan(&mut tokens, false)?;
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
        Some(Self { source, tokens, comment, base })
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
        for i in first..end {
            let t = tokens[i];
            match t.kind {
                Kind::Character(c @ (b'(' | b'[' | b'{')) => {
                    // A call's arguments and an index follow a value; an array and parens around an expression do not
                    let after_value = i > first && {
                        let p = tokens[i - 1];
                        matches!(p.kind, Kind::Identifier | Kind::Literal)
                            || p.is_char(b')')
                            || p.is_char(b']')
                            || p.is(Kind::Operator, "?.")
                            || (p.kind == Kind::Keyword
                                && matches!(
                                    p.text,
                                    "this" | "null" | "undefined" | "true" | "false"
                                ))
                    };
                    stack.push(Element::new(i, i, c == b'(' && !after_value, c == b'{'));
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
                    self.close(&element, i, all_of.is_some() && !kept, last_value, &mut found)?;
                }
                Kind::Character(b',') => {
                    if let Some(element) = stack.last_mut() {
                        let last_value = element.object && self.ends_object(i);
                        self.close(element, i, false, last_value, &mut found)?;
                        element.next(i);
                    }
                }
                Kind::Character(b':') => {
                    if let Some(element) = stack.last_mut()
                        && element.object
                        && element.start.is_none()
                    {
                        element.start = Some(i + 1);
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
                }
                _ => {}
            }
        }
        for Found { span, in_parens, last_value, spread, inner } in found {
            let expression = expression(allocator, self.source, inner)?;
            nested.push(NestedPipe { span, in_parens, last_value, spread, expression });
        }
        Some(nested)
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
        found.push(Found { span, in_parens, last_value, spread, inner });
        Some(())
    }
}

/// An outermost nested pipe found, read after the scan ([`Code::nested`]).
struct Found {
    span: Span,
    in_parens: bool,
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
}

impl Element {
    /// From the bracket `open`, its first element past `from` (an object's starts at a key's `:`).
    fn new(open: usize, from: usize, group: bool, object: bool) -> Self {
        Self {
            open,
            group,
            object,
            start: (!object).then_some(from + 1),
            pipe: false,
            conditional: false,
        }
    }

    /// The next one, after the `,` at `at`.
    fn next(&mut self, at: usize) {
        *self = Self::new(self.open, at, self.group, self.object);
    }
}

/// The action at `at` (an event binding's value) as Angular reads it (`parseAction`): its statements, between the root's `;`;
/// `None` where it reports an error the tokens show (no statement and no comment, a pipe, a bracket left open, `{{` and `}}` in it).
pub fn action<'a>(
    allocator: &'a Allocator,
    source: &str,
    at: Span,
) -> Option<ArenaBox<'a, AngularAction<'a>>> {
    let code = Code::read(source, at)?;
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
/// `None` where it reports an error the tokens show (no code, a `;`, a `:` no `?` or pipe takes, a pipe's name missing,
/// a bracket left open, `{{` and `}}` in it).
pub fn expression<'a>(
    allocator: &'a Allocator,
    source: &str,
    at: Span,
) -> Option<ArenaBox<'a, AngularExpression<'a>>> {
    let read = Code::read(source, at)?;
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

/// Where a comment starts in `s` (Angular's `_commentStart`): the first `//` outside quotes, `` ` `` one too.
fn comment_start(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut quote = None;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'/' && b[i + 1] == b'/' && quote.is_none() {
            return Some(i);
        }
        i += string_step(b[i], &mut quote, true).0;
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
}

impl<'s> Lexer<'s> {
    pub(super) fn new(s: &'s str) -> Self {
        Self { s, at: 0 }
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
            while let Some(b) = self.byte(self.at) {
                match b {
                    ..=b' ' => self.at += 1,
                    0xC2 if self.byte(self.at + 1) == Some(0xA0) => self.at += 2,
                    _ => break,
                }
            }
            let start = self.at;
            let Some(b) = self.byte(start) else { return (!in_template).then_some(()) };
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
                b'.' if self.byte(start + 1).is_some_and(|b| b.is_ascii_digit()) => {
                    self.number();
                    Kind::Literal
                }
                b'.' if self.s[start..].starts_with("...") => {
                    self.at += 3;
                    Kind::Operator
                }
                b'\'' | b'"' => {
                    self.string(b)?;
                    Kind::Literal
                }
                b'`' => {
                    self.at += 1;
                    self.template(tokens)?;
                    Kind::Literal
                }
                b'{' => {
                    braces += 1;
                    self.at += 1;
                    Kind::Character(b)
                }
                b'}' if in_template && braces == 0 => {
                    self.at += 1;
                    return Some(());
                }
                b'}' => {
                    braces -= 1;
                    self.at += 1;
                    Kind::Character(b)
                }
                b'(' | b')' | b'[' | b']' | b',' | b':' | b';' | b'.' => {
                    self.at += 1;
                    Kind::Character(b)
                }
                b'/' if starts_regex(&tokens[first..]) => {
                    self.regex()?;
                    Kind::Literal
                }
                _ => {
                    self.at = start + operator_len(&self.s.as_bytes()[start..])?;
                    Kind::Operator
                }
            };
            tokens.push(Token { kind, text: &self.s[start..self.at], start, end: self.at });
        }
    }

    fn skip(&self, from: usize, pred: impl Fn(u8) -> bool) -> usize {
        from + self.s.as_bytes()[from..].iter().take_while(|&&b| pred(b)).count()
    }

    /// Digits with `_`, a fraction and an exponent; Angular's errors in them are not repeated.
    fn number(&mut self) {
        self.at = self.skip(self.at, |b| b.is_ascii_digit() || matches!(b, b'_' | b'.'));
        if matches!(self.byte(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.byte(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.at = self.skip(self.at, |b| b.is_ascii_digit());
        }
    }

    fn string(&mut self, quote: u8) -> Option<()> {
        self.at += 1;
        loop {
            match self.byte(self.at)? {
                b'\\' => self.at += 2,
                b if b == quote => break,
                _ => self.at += 1,
            }
        }
        self.at += 1;
        Some(())
    }

    /// After a `` ` ``: through the closing one, its `${ }` scanned as tokens of their own (not kept).
    fn template(&mut self, tokens: &mut Vec<Token<'s>>) -> Option<()> {
        loop {
            match self.byte(self.at)? {
                b'\\' => self.at += 2,
                b'`' => break,
                b'$' if self.byte(self.at + 1) == Some(b'{') => {
                    self.at += 2;
                    let len = tokens.len();
                    self.scan(tokens, true)?;
                    tokens.truncate(len);
                }
                _ => self.at += 1,
            }
        }
        self.at += 1;
        Some(())
    }

    fn regex(&mut self) -> Option<()> {
        self.at += 1;
        let mut class = false;
        loop {
            match self.byte(self.at)? {
                b'\\' => self.at += 1,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => break,
                _ => {}
            }
            self.at += 1;
        }
        self.at = self.skip(self.at + 1, |b| b.is_ascii_alphabetic());
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

/// The length of the operator Angular's lexer reads at the start of `s`; `None` for a character it reports.
fn operator_len(s: &[u8]) -> Option<usize> {
    // Longest first
    const OPERATORS: [&[u8]; 36] = [
        b"===", b"!==", b"**=", b"??=", b"&&=", b"||=", b"==", b"!=", b"<=", b">=", b"=>", b"&&",
        b"||", b"??", b"?.", b"**", b"+=", b"-=", b"*=", b"/=", b"%=", b"&=", b"|=", b"+", b"-",
        b"*", b"/", b"%", b"<", b">", b"=", b"!", b"?", b"&", b"|", b"^",
    ];
    OPERATORS.iter().find(|op| s.starts_with(op)).map(|op| op.len())
}
