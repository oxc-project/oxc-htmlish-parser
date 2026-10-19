//! `sig-batch <html|vue>`: reads a JSON array of sources on stdin, prints a JSON array with,
//! per source, its structural signature or the error message as a string.
//! The subprocess bridge for `tasks/differential/run.ts`, which builds the same shape from the oracle.
//!
//! - element: `["el", name, flags, attrs, children]`, flags from `/` (written `/>`), `e` (end tag), `lf` (ignored first line break)
//! - attribute: `[name, value | null]`, plus `dir, arg | null, dynamic, modifiers` for a Vue directive
//! - `["t" | "i" | "d" | "x", raw]`: text, interpolation, doctype, CDATA
//! - `["c", content]`: comment of any kind, without its delimiters

use oxc_htmlish_parser::{
    Allocator, Dialect, Parser,
    ast::{AttributeKind, Element, Node},
};
use serde_json::{Value, json};

fn main() {
    let dialect = match std::env::args().nth(1).as_deref() {
        Some("vue") => Dialect::Vue,
        _ => Dialect::Html,
    };
    let buf = std::io::read_to_string(std::io::stdin()).unwrap();
    let inputs: Vec<String> = serde_json::from_str(&buf).expect("JSON array of strings");
    let mut allocator = Allocator::default();
    let outputs: Vec<Value> = inputs
        .iter()
        .map(|source| {
            let sig = match Parser::new(&allocator, source, dialect).parse() {
                Ok(root) => nodes(source, &root.children),
                Err(diagnostic) => Value::String(diagnostic.to_string()),
            };
            allocator.reset();
            sig
        })
        .collect();
    serde_json::to_writer(std::io::stdout().lock(), &outputs).unwrap();
}

fn nodes(source: &str, nodes: &[Node<'_>]) -> Value {
    nodes.iter().map(|node| self::node(source, node)).collect()
}

fn node(source: &str, node: &Node<'_>) -> Value {
    let raw = |kind: &str| json!([kind, node.span().slice(source)]);
    match node {
        Node::Element(el) => element(source, el),
        Node::Text(_) => raw("t"),
        Node::Interpolation(_) => raw("i"),
        Node::Comment(n) => json!(["c", n.content.slice(source)]),
        Node::ConditionalComment(_) | Node::ConditionalStart(_) | Node::ConditionalEnd(_) => {
            let raw = node.span().slice(source);
            json!(["c", &raw[4..raw.len() - 3]])
        }
        Node::DocType(_) => raw("d"),
        Node::Cdata(_) => raw("x"),
    }
}

fn element(source: &str, el: &Element<'_>) -> Value {
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
    json!(["el", el.name.slice(source), flags, attrs, nodes(source, &el.children)])
}
