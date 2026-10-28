//! Vue directive names, split as `@vue/compiler-core`'s tokenizer does.

use oxc_allocator::{Allocator, ArenaVec};

use crate::{
    ast::{VueArgument, VueDirective},
    span::{Span, span},
};

/// A directive name starts here (`v-`, or a shorthand), as Vue's tokenizer tells them from plain attributes.
pub fn is_directive_start(name: &[u8]) -> bool {
    name.starts_with(b"v-") || matches!(name.first(), Some(b':' | b'@' | b'#' | b'.'))
}

/// `None` for plain attributes and malformed directive names.
pub fn directive<'a>(
    allocator: &'a Allocator,
    source: &str,
    name: Span,
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
        let end = s[2..].iter().position(|&b| matches!(b, b':' | b'.')).map_or(s.len(), |i| 2 + i);
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
        // Slots have no modifiers: the dots belong to the argument
        let end = if slot { s.len() } else { until_dot(static_from) };
        // Without one (`v-bind:`, a lone `:`), Vue reads no argument (`:="o"` binds an object)
        if end > from {
            argument = Some(VueArgument { span: at(from, end), dynamic });
        }
        cursor = end;
    }

    let mut modifiers = ArenaVec::new_in(&allocator);
    while cursor < s.len() {
        let end = until_dot(cursor + 1);
        modifiers.push(at(cursor + 1, end));
        cursor = end;
    }
    Some(VueDirective { name: at(dir.0, dir.1), argument, modifiers })
}
