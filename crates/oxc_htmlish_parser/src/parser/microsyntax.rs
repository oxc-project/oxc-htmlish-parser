//! Angular's microsyntax (`*ngFor="let item of items; trackBy: f"`), split as its compiler reads it (`parseTemplateBindings`):
//! its lexer's tokens, and an expression ending where its parser stops (`parsePipe`), which takes no JS parse:
//! after a value, only an operator, `(`, `[`, `.`, a template literal, or the `:` of a conditional or a pipe's argument continues it.

use oxc_allocator::{Allocator, ArenaVec};

use crate::{
    ast::TemplateBinding,
    span::{Span, span},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
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
struct Token<'s> {
    kind: Kind,
    text: &'s str,
    start: usize,
    end: usize,
}

impl Token<'_> {
    fn is(&self, kind: Kind, text: &str) -> bool {
        self.kind == kind && self.text == text
    }

    fn is_char(&self, c: u8) -> bool {
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

/// `value` (an attribute's value) split into its bindings, as written; `None` where Angular reports an error.
pub fn template_bindings<'a>(
    allocator: &'a Allocator,
    source: &str,
    value: Span,
) -> Option<ArenaVec<'a, TemplateBinding>> {
    let s = value.slice(source);
    let mut tokens = vec![];
    Lexer { s, at: 0 }.scan(&mut tokens, false)?;
    let base = value.start as usize;
    let mut parser = Bindings { tokens: &tokens, at: 0, base, out: ArenaVec::new_in(&allocator) };
    parser.directive_bindings(None)?;
    while parser.at < tokens.len() {
        if parser.peek_keyword("let") {
            parser.at += 1;
            let name = parser.key()?;
            let value = if parser.peek().is_some_and(|t| t.is(Kind::Operator, "=")) {
                parser.at += 1;
                Some(parser.key()?)
            } else {
                None
            };
            parser.out.push(TemplateBinding::Let { name, value });
            parser.terminator();
        } else {
            let key = parser.key()?;
            if !parser.as_binding(Some(key))? {
                parser.directive_bindings(Some(key))?;
            }
        }
        parser.terminator();
    }
    Some(parser.out)
}

struct Lexer<'s> {
    s: &'s str,
    at: usize,
}

impl<'s> Lexer<'s> {
    fn byte(&self, at: usize) -> Option<u8> {
        self.s.as_bytes().get(at).copied()
    }

    /// Tokens up to the end, or, `in_template`, up to the `}` closing a template literal's `${`, which it consumes.
    fn scan(&mut self, tokens: &mut Vec<Token<'s>>, in_template: bool) -> Option<()> {
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

struct Bindings<'t, 's, 'a> {
    tokens: &'t [Token<'s>],
    at: usize,
    base: usize,
    out: ArenaVec<'a, TemplateBinding>,
}

impl Bindings<'_, '_, '_> {
    fn peek(&self) -> Option<&Token<'_>> {
        self.tokens.get(self.at)
    }

    fn peek_keyword(&self, word: &str) -> bool {
        self.peek().is_some_and(|t| t.is(Kind::Keyword, word))
    }

    fn span(&self, first: usize, last: usize) -> Span {
        span(self.base + self.tokens[first].start, self.base + self.tokens[last].end)
    }

    /// An optional `;` or `,`.
    fn terminator(&mut self) {
        if self.peek().is_some_and(|t| t.is_char(b';') || t.is_char(b',')) {
            self.at += 1;
        }
    }

    /// Identifiers, keywords or strings joined by `-` (`expectTemplateBindingKey`).
    fn key(&mut self) -> Option<Span> {
        let first = self.at;
        loop {
            let t = self.peek()?;
            if !(matches!(t.kind, Kind::Identifier | Kind::Keyword)
                || (t.kind == Kind::Literal && matches!(t.text.as_bytes()[0], b'\'' | b'"')))
            {
                return None;
            }
            self.at += 1;
            if !self.peek().is_some_and(|t| t.is(Kind::Operator, "-")) {
                return Some(self.span(first, self.at - 1));
            }
            self.at += 1;
        }
    }

    /// `as alias` after `key`'s binding (`parseAsBinding`); whether there is one.
    fn as_binding(&mut self, key: Option<Span>) -> Option<bool> {
        if !self.peek_keyword("as") {
            return Some(false);
        }
        self.at += 1;
        let alias = self.key()?;
        self.out.push(TemplateBinding::As { key, alias });
        self.terminator();
        Some(true)
    }

    /// `key`'s `:`, expression and `as alias` (`parseDirectiveKeywordBindings`); `key` `None` for the directive's own.
    fn directive_bindings(&mut self, key: Option<Span>) -> Option<()> {
        if self.peek().is_some_and(|t| t.is_char(b':')) {
            self.at += 1;
        }
        let expression = if self.at == self.tokens.len()
            || self.peek_keyword("as")
            || self.peek_keyword("let")
        {
            None
        } else {
            let first = self.at;
            self.at = expression_end(self.tokens, first)?;
            Some(self.span(first, self.at - 1))
        };
        self.out.push(TemplateBinding::Expression { key, expression });
        if !self.as_binding(key)? {
            self.terminator();
        }
        Some(())
    }
}

/// Where the expression starting at token `first` ends (`parsePipe`): the first token past it at its depth
/// that no operator, call, member access, template literal, conditional or pipe argument takes;
/// `None` where Angular reports it (an operand missing, a bracket left open or never opened).
fn expression_end(tokens: &[Token<'_>], first: usize) -> Option<usize> {
    let mut open: Vec<u8> = vec![];
    let (mut operand, mut conditionals, mut pipe) = (true, 0, false);
    for (i, t) in tokens.iter().enumerate().skip(first) {
        if let Kind::Character(c @ (b'(' | b'[' | b'{')) = t.kind {
            open.push(c);
            continue;
        }
        if let Kind::Character(c @ (b')' | b']' | b'}')) = t.kind {
            let opener = match c {
                b')' => b'(',
                b']' => b'[',
                _ => b'{',
            };
            if open.pop()? != opener {
                return None;
            }
            operand = false;
            continue;
        }
        if !open.is_empty() {
            continue;
        }
        if operand {
            // A prefix operator keeps waiting for the operand
            operand = matches!(t.kind, Kind::Operator if matches!(t.text, "!" | "-" | "+"))
                || t.is(Kind::Keyword, "typeof")
                || t.is(Kind::Keyword, "void");
            if !operand && !matches!(t.kind, Kind::Identifier | Kind::Keyword | Kind::Literal) {
                return None;
            }
            continue;
        }
        match t.kind {
            // A tagged template, and `x!`
            Kind::Literal if t.text.starts_with('`') => {}
            Kind::Operator if t.text == "!" => {}
            Kind::Operator => {
                conditionals += usize::from(t.text == "?");
                pipe |= t.text == "|";
                operand = true;
            }
            Kind::Character(b'.') => operand = true,
            Kind::Character(b':') if conditionals > 0 => {
                conditionals -= 1;
                operand = true;
            }
            Kind::Character(b':') if pipe => operand = true,
            Kind::Keyword if matches!(t.text, "in" | "instanceof") => operand = true,
            _ => return (conditionals == 0).then_some(i),
        }
    }
    (open.is_empty() && !operand && conditionals == 0).then_some(tokens.len())
}
