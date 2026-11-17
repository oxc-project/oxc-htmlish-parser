//! Angular's `@for` and `@if` parameters, split as its compiler reads them
//! (`parseForLoopParameters`, `parseConditionalBlockParameters`).

use oxc_allocator::{Allocator, ArenaVec};

use super::{is_identifier_part, is_js_space, skip_js_space, trim_js};
use crate::{
    ast::{BlockKind, BlockParameter, BlockParameterKind, LetVariable},
    span::{Span, span},
};

/// The parameter at `index` of a `block`, `parameter` its trimmed span.
pub fn parameter<'a>(
    allocator: &'a Allocator,
    source: &str,
    block: BlockKind,
    index: usize,
    parameter: Span,
) -> BlockParameter<'a> {
    let s = parameter.slice(source);
    let base = parameter.start as usize;
    let at = |(start, end): (usize, usize)| span(base + start, base + end);
    let kind = match (block, index) {
        (BlockKind::For, 0) => for_of(s).map(|(item, expression)| BlockParameterKind::ForOf {
            item: at(item),
            expression: at(expression),
        }),
        // `let` first, as Angular tries it
        (BlockKind::For, _) => match after_keyword(s, "let") {
            Some(from) => let_variables(allocator, s, from, at).map(BlockParameterKind::Let),
            None => after_keyword(s, "track").map(|from| BlockParameterKind::Track {
                expression: at(trim_js(s, (from, s.len()))),
            }),
        },
        (BlockKind::If | BlockKind::ElseIf, 1..) => after_keyword(s, "as")
            .map(|from| trim_js(s, (from, s.len())))
            .filter(|&(start, end)| is_identifier(&s[start..end]))
            .map(|alias| BlockParameterKind::As { alias: at(alias) }),
        _ => None,
    };
    let kind = kind.unwrap_or(BlockParameterKind::Plain);
    let expression = match (&kind, block, index) {
        (
            BlockParameterKind::Plain,
            BlockKind::If | BlockKind::ElseIf | BlockKind::Switch | BlockKind::Case,
            0,
        ) => Some(parameter),
        (
            BlockParameterKind::ForOf { expression, .. } | BlockParameterKind::Track { expression },
            ..,
        ) => Some(*expression),
        _ => None,
    };
    let angular =
        expression.and_then(|e| super::angular_expression::expression(allocator, source, e));
    BlockParameter { span: parameter, kind, angular }
}

/// `/^word\s+/` at the start of `s`: where the rest starts.
fn after_keyword(s: &str, word: &str) -> Option<usize> {
    let rest = s.strip_prefix(word)?;
    let from = s.len() - rest.len();
    let after = skip_js_space(s, from);
    (after > from).then_some(after)
}

/// `/^\s*([0-9A-Za-z_$]*)\s+of\s+([\S\s]*)/` inside the parentheses Angular strips (`stripOptionalParentheses`):
/// the item and the expression; `None` without an item.
fn for_of(s: &str) -> Option<((usize, usize), (usize, usize))> {
    let (start, end) = strip_parentheses(s)?;
    let t = &s[..end];
    let item_start = skip_js_space(t, start);
    let item_end =
        item_start + t[item_start..].bytes().take_while(|&b| is_identifier_part(b)).count();
    let of = skip_js_space(t, item_end);
    if item_end == item_start || of == item_end {
        return None;
    }
    let expression = trim_js(t, (of + after_keyword(&t[of..], "of")?, t.len()));
    (expression.0 < expression.1).then_some(((item_start, item_end), expression))
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
        let name = trim_js(s, (part_start, part_start + name.len()));
        let value_start = part_start + part.len() - value.len();
        let value = trim_js(s, (value_start, value_start + value.len()));
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
    s.bytes().next().is_some_and(|b| !b.is_ascii_digit()) && s.bytes().all(is_identifier_part)
}
