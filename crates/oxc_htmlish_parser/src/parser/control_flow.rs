//! Angular's `@for` and `@if` parameters, split as its compiler reads them
//! (`parseForLoopParameters`, `parseConditionalBlockParameters`).

use oxc_allocator::{Allocator, ArenaVec};

use super::vue::{is_js_space, skip_space, trim};
use crate::{
    ast::{BlockParameter, BlockParameterKind, LetVariable},
    span::{Span, span},
};

/// The blocks whose parameters Angular splits.
#[derive(Clone, Copy)]
pub enum Reading {
    For,
    If,
    Other,
}

impl Reading {
    /// `name` as written (`else   if`): Angular collapses whitespace without a line break.
    pub fn of(name: &str) -> Self {
        if name == "for" {
            Self::For
        } else if !name.contains(['\n', '\r'])
            && matches!(
                name.split_ascii_whitespace().collect::<Vec<_>>()[..],
                ["if"] | ["else", "if"]
            )
        {
            Self::If
        } else {
            Self::Other
        }
    }
}

/// The parameter at `index` of a block read as `reading`, `parameter` its trimmed span.
pub fn parameter<'a>(
    allocator: &'a Allocator,
    source: &str,
    reading: Reading,
    index: usize,
    parameter: Span,
) -> BlockParameter<'a> {
    let s = parameter.slice(source);
    let base = parameter.start as usize;
    let at = |(start, end): (usize, usize)| span(base + start, base + end);
    let kind = match (reading, index) {
        (Reading::For, 0) => for_of(s).map(|(item, expression)| BlockParameterKind::ForOf {
            item: at(item),
            expression: at(expression),
        }),
        // `let` first, as Angular tries it
        (Reading::For, _) => match (after_keyword(s, "let"), after_keyword(s, "track")) {
            (Some(from), _) => let_variables(allocator, s, from, at).map(BlockParameterKind::Let),
            (None, Some(from)) => {
                Some(BlockParameterKind::Track { expression: at(trim(s, (from, s.len()))) })
            }
            (None, None) => None,
        },
        (Reading::If, 1..) => after_keyword(s, "as")
            .map(|from| trim(s, (from, s.len())))
            .filter(|&(start, end)| is_identifier(&s[start..end]))
            .map(|alias| BlockParameterKind::As { alias: at(alias) }),
        _ => None,
    };
    BlockParameter { span: parameter, kind: kind.unwrap_or(BlockParameterKind::Plain) }
}

/// `/^word\s+/` at the start of `s`: where the rest starts.
fn after_keyword(s: &str, word: &str) -> Option<usize> {
    let rest = s.strip_prefix(word)?;
    let from = s.len() - rest.len();
    let after = skip_space(s, from);
    (after > from).then_some(after)
}

/// `/^\s*([0-9A-Za-z_$]*)\s+of\s+([\S\s]*)/` inside the parentheses Angular strips (`stripOptionalParentheses`):
/// the item and the expression; `None` without an item.
fn for_of(s: &str) -> Option<((usize, usize), (usize, usize))> {
    let (start, end) = strip_parentheses(s)?;
    let t = &s[..end];
    let item_start = skip_space(t, start);
    let item_end = item_start
        + t[item_start..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
            .count();
    let of = skip_space(t, item_end);
    if item_end == item_start || of == item_end || !t[of..].starts_with("of") {
        return None;
    }
    let expression = skip_space(t, of + 2);
    let expression = trim(t, (expression, t.len()));
    (expression.0 > of + 2 && expression.0 < expression.1)
        .then_some(((item_start, item_end), expression))
}

/// The parentheses before and after `s`'s content, past whitespace, as many on each side: `(start, end)` inside them.
fn strip_parentheses(s: &str) -> Option<(usize, usize)> {
    let mut open = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => {
                start = i + 1;
                open += 1;
            }
            c if is_js_space(c) => {}
            _ => break,
        }
    }
    let mut end = s.len();
    for (i, c) in s.char_indices().rev() {
        if open == 0 {
            break;
        }
        match c {
            ')' => {
                end = i;
                open -= 1;
            }
            c if is_js_space(c) => {}
            _ => break,
        }
    }
    (open == 0 && start <= end).then_some((start, end))
}

/// `name = variable, …` from `from`: split at `,`, then at `=`, each part trimmed.
fn let_variables<'a>(
    allocator: &'a Allocator,
    s: &str,
    from: usize,
    at: impl Fn((usize, usize)) -> Span,
) -> Option<ArenaVec<'a, LetVariable>> {
    let mut variables = ArenaVec::new_in(&allocator);
    let mut part_start = from;
    for part in s[from..].split(',') {
        let mut halves = part.splitn(3, '=');
        let (Some(name), Some(value), None) = (halves.next(), halves.next(), halves.next()) else {
            return None;
        };
        let name = trim(s, (part_start, part_start + name.len()));
        let value_start = part_start + part.len() - value.len();
        let value = trim(s, (value_start, value_start + value.len()));
        if !is_identifier(&s[name.0..name.1]) || value.0 == value.1 {
            return None;
        }
        variables.push(LetVariable { name: at(name), value: at(value) });
        part_start += part.len() + 1;
    }
    Some(variables)
}

/// `/^[$A-Z_][0-9A-Z_$]*$/i`.
fn is_identifier(s: &str) -> bool {
    s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'$' | b'_'))
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_'))
}
