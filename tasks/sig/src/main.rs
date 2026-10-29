//! `sig-batch <html|vue|angular>`: reads a JSON array of sources on stdin, prints a JSON array with,
//! per source, its structural signature, the error message as a string,
//! or `"invariant: …"` when the AST breaks one (`tests/check`).
//! The subprocess bridge for `tasks/differential/run.ts`, which builds the same shape from the oracle.
//!
//! - element: `["el", name, flags, attrs, children]`, flags from `/` (written `/>`), `e` (end tag), `lf` (ignored first line break)
//! - attribute: `[name, value | null]`, plus `dir, arg | null, dynamic, modifiers` for a Vue directive
//! - `["t" | "i" | "d" | "x", raw]`: text, interpolation, doctype, CDATA
//! - `["c", content]`: comment, without its delimiters; `["bc", content, opening]` a bogus one (`<!x>`, `<?x>`, `</ x>`)
//! - `["b", name, parameters, children]`: Angular block

#[path = "../../../crates/oxc_htmlish_parser/tests/check/mod.rs"]
mod check;

use oxc_htmlish_parser::{
    Allocator, Dialect, Parser,
    ast::{AttributeKind, Element, NodeId, NodeKind, Root},
};
use serde_json::{Value, json};

fn main() {
    let dialect = match std::env::args().nth(1).as_deref() {
        Some("html") => Dialect::Html,
        Some("vue") => Dialect::Vue,
        Some("angular") => Dialect::Angular,
        other => panic!("unknown dialect {other:?}, expected html, vue or angular"),
    };
    let buf = std::io::read_to_string(std::io::stdin()).unwrap();
    let inputs: Vec<String> = serde_json::from_str(&buf).expect("JSON array of strings");
    let mut allocator = Allocator::default();
    let outputs: Vec<Value> = inputs
        .iter()
        .map(|source| {
            let sig = match Parser::new(&allocator, source, dialect).parse() {
                Ok(root) => match check::check(&root, NodeId::DOCUMENT) {
                    Ok(()) => children(source, &root, NodeId::DOCUMENT),
                    Err(broken) => Value::String(format!("invariant: {broken}")),
                },
                Err(diagnostic) => Value::String(diagnostic.to_string()),
            };
            allocator.reset();
            sig
        })
        .collect();
    serde_json::to_writer(std::io::stdout().lock(), &outputs).unwrap();
}

fn children(source: &str, root: &Root<'_>, id: NodeId) -> Value {
    root.children(id).map(|(child, _)| node(source, root, child)).collect()
}

fn node(source: &str, root: &Root<'_>, id: NodeId) -> Value {
    let node = &root[id];
    let raw = |kind: &str| json!([kind, node.span.slice(source)]);
    match &node.kind {
        NodeKind::Document => unreachable!(),
        NodeKind::Element(el) => element(source, root, id, el),
        NodeKind::Text(_) => raw("t"),
        NodeKind::Interpolation(_) => raw("i"),
        // A bogus comment keeps how it opened (`<!`, `<?`, `</`): Vue drops some kinds only.
        NodeKind::Comment(n) if n.bogus => {
            json!(["bc", n.content.slice(source), &node.span.slice(source)[..2]])
        }
        NodeKind::Comment(n) => json!(["c", n.content.slice(source)]),
        NodeKind::ConditionalComment(_)
        | NodeKind::ConditionalStart(_)
        | NodeKind::ConditionalEnd => {
            let raw = node.span.slice(source);
            json!(["c", &raw[4..raw.len() - 3]])
        }
        NodeKind::DocType(_) => raw("d"),
        NodeKind::Cdata => raw("x"),
        // Never compared: the oracles report it
        NodeKind::EndTag(_) => raw("e"),
        NodeKind::Block(b) => {
            let parameters: Vec<&str> = b.parameters.iter().map(|p| p.slice(source)).collect();
            json!(["b", b.name.slice(source), parameters, children(source, root, id)])
        }
    }
}

fn element(source: &str, root: &Root<'_>, id: NodeId, el: &Element<'_>) -> Value {
    let mut flags = String::new();
    if el.self_closing {
        flags.push('/');
    }
    if el.end_tag.is_some() {
        flags.push('e');
    }
    if el.leading_lf {
        flags.push_str("lf");
    }
    let attrs: Vec<Value> = el
        .attributes
        .iter()
        .map(|a| {
            let name = a.name.slice(source);
            let value = a.value.map(|v| v.span.slice(source));
            match &a.kind {
                AttributeKind::Plain => json!([name, value]),
                AttributeKind::Vue(d) => {
                    let shorthand = d.name.slice(source);
                    let dir = match shorthand {
                        ":" | "." => "bind",
                        "@" => "on",
                        "#" => "slot",
                        name => name,
                    };
                    // Vue spells `.prop` as a `prop` modifier.
                    let modifiers: Vec<&str> = (shorthand == ".")
                        .then_some("prop")
                        .into_iter()
                        .chain(d.modifiers.iter().map(|m| m.slice(source)))
                        .collect();
                    let arg = d.argument.map(|arg| arg.span.slice(source));
                    let dynamic = d.argument.is_some_and(|arg| arg.dynamic);
                    json!([name, value, dir, arg, dynamic, modifiers])
                }
            }
        })
        .collect();
    json!(["el", el.name.slice(source), flags, attrs, children(source, root, id)])
}
