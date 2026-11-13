//! Vue directive names, split as `@vue/compiler-core`'s tokenizer does, and `v-for`'s value as its parser does.

use oxc_allocator::{Allocator, ArenaBox, ArenaVec};

use crate::{
    ast::{VueArgument, VueDirective, VueFor},
    span::{Span, span},
};

/// A directive name starts here (`v-`, or a shorthand), as Vue's tokenizer tells them from plain attributes.
pub fn is_directive_start(name: &[u8]) -> bool {
    name.starts_with(b"v-") || matches!(name.first(), Some(b':' | b'@' | b'#' | b'.'))
}

/// `v-pre`, with an argument or modifiers too (`v-pre:a`), which Vue reads as `v-pre`: its name is `pre`.
pub fn is_v_pre(name: &[u8]) -> bool {
    name.starts_with(b"v-") && &name[2..v_name_end(name)] == b"pre"
}

/// Where the name of a `v-` directive `s` ends: at its argument's `:`, its first modifier's `.`, or the end.
fn v_name_end(s: &[u8]) -> usize {
    s[2..].iter().position(|&b| matches!(b, b':' | b'.')).map_or(s.len(), |i| 2 + i)
}

/// `None` for plain attributes and malformed directive names.
pub fn directive<'a>(
    allocator: &'a Allocator,
    source: &str,
    name: Span,
    value: Option<Span>,
) -> Option<VueDirective<'a>> {
    let s = name.slice(source).as_bytes();
    let base = name.start as usize;
    let at = |start: usize, end: usize| span(base + start, base + end);
    let until_dot =
        |from: usize| s[from..].iter().position(|&b| b == b'.').map_or(s.len(), |i| from + i);

    // `v-name`, then `:arg` and/or `.modifiers`; or a shorthand followed by the argument
    if !is_directive_start(s) {
        return None;
    }
    let (dir, arg_from, mut cursor) = if s.starts_with(b"v-") {
        let end = v_name_end(s);
        if end == 2 {
            return None;
        }
        let arg_from = (s.get(end) == Some(&b':')).then_some(end + 1);
        ((2, end), arg_from, end)
    } else {
        ((0, 1), Some(1), 1)
    };
    let slot = matches!(&s[dir.0..dir.1], b"#" | b"slot");

    let mut argument = None;
    if let Some(from) = arg_from {
        let dynamic = s.get(from) == Some(&b'[');
        let static_from =
            if dynamic { from + s[from..].iter().position(|&b| b == b']')? + 1 } else { from };
        let end = until_dot(static_from);
        // Without one (`v-bind:`, a lone `:`, `#.a`), Vue reads no argument (`:="o"` binds an object).
        // Slots have no modifiers: the dots belong to the argument, or go with none
        if end > from {
            argument =
                Some(VueArgument { span: at(from, if slot { s.len() } else { end }), dynamic });
        }
        cursor = end;
    }

    let mut modifiers = ArenaVec::new_in(&allocator);
    while !slot && cursor < s.len() {
        let end = until_dot(cursor + 1);
        modifiers.push(at(cursor + 1, end));
        cursor = end;
    }
    let for_expression = value
        .filter(|_| &s[dir.0..dir.1] == b"for")
        .and_then(|value| for_expression(source, value))
        .map(|parts| ArenaBox::new_in(parts, &allocator));
    Some(VueDirective { name: at(dir.0, dir.1), argument, modifiers, for_expression })
}

/// `v-for`'s value split as Vue's `parseForExpression` (see [`VueFor`]).
fn for_expression(source: &str, value: Span) -> Option<VueFor> {
    let s = value.slice(source);
    let at = |(start, end): (usize, usize)| {
        span(value.start as usize + start, value.start as usize + end)
    };
    let part = |range: (usize, usize)| Some(at(trim(s, range))).filter(|part| !part.is_empty());

    // `/([\s\S]*?)\s+(?:in|of)\s+(\S[\s\S]*)/`: the first whitespace run before `in` / `of`, whitespace and more.
    // Every position in a run reaches the same `in` / `of`, so only its first is tried
    let (lhs_end, operator, rhs) = s.char_indices().find_map(|(i, c)| {
        if !is_js_space(c) || s[..i].ends_with(is_js_space) {
            return None;
        }
        let operator = skip_space(s, i);
        if !(s[operator..].starts_with("in") || s[operator..].starts_with("of")) {
            return None;
        }
        let rhs = skip_space(s, operator + 2);
        (rhs > operator + 2 && rhs < s.len()).then_some((i, operator, rhs))
    })?;

    // `LHS.trim().replace(/^\(|\)$/g, '').trim()`
    let (mut start, mut end) = trim(s, (0, lhs_end));
    start += usize::from(s[start..end].starts_with('('));
    end -= usize::from(s[start..end].ends_with(')'));
    let aliases = trim(s, (start, end));

    // `/,([^,\}\]]*)(?:,([^,\}\]]*))?$/`: the last one or two commas with no `]` / `}` after them
    let (start, end) = aliases;
    let tail = s[start..end].rfind([']', '}']).map_or(start, |i| start + i + 1);
    let mut commas = s[tail..end].rmatch_indices(',').map(|(i, _)| tail + i);
    let (value, key, index) = match (commas.next(), commas.next()) {
        (None, _) => (part(aliases), None, None),
        (Some(last), None) => (part((start, last)), part((last + 1, end)), None),
        (Some(last), Some(first)) => {
            (part((start, first)), part((first + 1, last)), part((last + 1, end)))
        }
    };
    Some(VueFor {
        aliases: at(aliases),
        value,
        key,
        index,
        operator: at((operator, operator + 2)),
        source: at(trim(s, (rhs, s.len()))),
    })
}

/// JS's `\s` and `trim()`: Unicode's White_Space without U+0085, and U+FEFF.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn skip_space(s: &str, from: usize) -> usize {
    s.len() - s[from..].trim_start_matches(is_js_space).len()
}

/// `range` of `s` without JS whitespace at its edges.
fn trim(s: &str, (start, end): (usize, usize)) -> (usize, usize) {
    let part = &s[start..end];
    let lead = part.len() - part.trim_start_matches(is_js_space).len();
    let trail = part.trim_end_matches(is_js_space).len();
    (start + lead, start + lead.max(trail))
}
