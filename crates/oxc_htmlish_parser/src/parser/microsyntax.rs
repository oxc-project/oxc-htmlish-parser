//! Angular's microsyntax (`*ngFor="let item of items; trackBy: f"`), split as its compiler reads it (`parseTemplateBindings`),
//! on its expression lexer's tokens (`angular_expression.rs`): an expression ends where its parser stops (`parsePipe`):
//! after a value, only an operator, `(`, `[`, `.`, a template literal, or the `:` of a conditional or a pipe's argument continues it.

use oxc_allocator::{Allocator, ArenaVec};

use super::angular_expression::{Kind, Lexer, Token};
use crate::{
    ast::TemplateBinding,
    span::{Span, span},
};

/// `value` (an attribute's value) split into its bindings, as written; `None` where Angular reports an error.
pub fn template_bindings<'a>(
    allocator: &'a Allocator,
    source: &str,
    value: Span,
) -> Option<ArenaVec<'a, TemplateBinding<'a>>> {
    let s = value.slice(source);
    let mut tokens = vec![];
    Lexer::new(s, true).scan(&mut tokens, false)?;
    let base = value.start as usize;
    let mut parser = Bindings {
        allocator,
        source,
        tokens: &tokens,
        at: 0,
        base,
        out: ArenaVec::new_in(&allocator),
    };
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
            if parser.peek_keyword("as") {
                let alias = parser.alias()?;
                parser.out.push(TemplateBinding::As { key, alias });
            } else {
                parser.directive_bindings(Some(key))?;
            }
        }
        parser.terminator();
    }
    Some(parser.out)
}

struct Bindings<'t, 's, 'a> {
    allocator: &'a Allocator,
    source: &'s str,
    tokens: &'t [Token<'s>],
    at: usize,
    base: usize,
    out: ArenaVec<'a, TemplateBinding<'a>>,
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
            if !(matches!(t.kind, Kind::Identifier | Kind::Keyword) || t.is_string()) {
                return None;
            }
            self.at += 1;
            if !self.peek().is_some_and(|t| t.is(Kind::Operator, "-")) {
                return Some(self.span(first, self.at - 1));
            }
            self.at += 1;
        }
    }

    /// `as alias` at an `as` (`parseAsBinding`).
    fn alias(&mut self) -> Option<Span> {
        self.at += 1;
        let alias = self.key()?;
        self.terminator();
        Some(alias)
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
        let angular = expression.and_then(|e| {
            super::angular_expression::expression(self.allocator, self.source, e, true)
        });
        let alias = if self.peek_keyword("as") {
            Some(self.alias()?)
        } else {
            self.terminator();
            None
        };
        self.out.push(TemplateBinding::Expression { key, expression, angular, alias });
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
