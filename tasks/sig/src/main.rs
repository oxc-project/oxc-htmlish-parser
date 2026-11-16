//! `sig-batch <html|vue|angular>`: reads a JSON array of sources on stdin, prints a JSON array with,
//! per source, its structural signature, the error message as a string,
//! or `"invariant: …"` when the AST breaks one (`tests/check`).
//! The subprocess bridge for `tasks/differential/run.ts`, which builds the same shape from the oracle.
//!
//! - element: `["el", name, flags, attrs, children]`, flags from `/` (written `/>`), `e` (end tag), `lf` (ignored first line break)
//! - attribute: `[name, value | null]`, plus `dir, arg | null, dynamic, modifiers, for` for a Vue directive,
//!   `for` being `[value, key, index, source]` (each `null` when missing) or `null`;
//!   `["comment", raw]` for an Angular start-tag comment; an Angular value's `{{ }}` expressions follow it when it has any,
//!   and a `*x` value's `["tb", bindings | null]`, each `["e", key, expression]`, `["l", name, value]` or `["a", key, alias]`
//! - `["t" | "i" | "d" | "x", raw]`: text, interpolation, doctype, CDATA
//! - `["c", content]`: comment, without its delimiters; `["bc", content, opening]` a bogus one (`<!x>`, `<?x>`, `</ x>`)
//! - `["b", name, parameters, children, reading]`: Angular block, `name` its kind's (as written when unknown),
//!   `reading` its parameters' split
//!   (`[item, expression, track, [[name, value], …], alias]`, each `null` when none)
//! - `["l", name, value]`: Angular `@let`
//! - `["icu", expression, type, cases]`, `["case", value, children]`: Angular ICU expression

#[path = "../../../crates/oxc_htmlish_parser/tests/check/mod.rs"]
mod check;

use oxc_htmlish_parser::{
    Allocator, Dialect, Parser,
    ast::{
        AngularAttribute, AttributeKind, Block, BlockParameterKind, Element, NodeId, NodeKind,
        Root, TemplateBinding,
    },
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
            let parameters: Vec<&str> = b.parameters.iter().map(|p| p.span.slice(source)).collect();
            // As Angular names it (`Block::kind`), or as written
            let name = b.kind.name().unwrap_or_else(|| b.name.slice(source));
            json!(["b", name, parameters, children(source, root, id), block_reading(source, b)])
        }
        NodeKind::LetDeclaration(n) => json!(["l", n.name.slice(source), n.value.slice(source)]),
        NodeKind::Icu(n) => json!([
            "icu",
            n.expression.slice(source),
            n.type_name.slice(source),
            children(source, root, id)
        ]),
        NodeKind::IcuCase(n) => json!(["case", n.value.slice(source), children(source, root, id)]),
    }
}

/// `[item, expression, track, [[name, value], …], alias]` from a block's parameters, each `null` when none.
fn block_reading(source: &str, block: &Block<'_>) -> Value {
    let (mut item, mut expression, mut track, mut lets, mut alias) =
        (None, None, None, vec![], None);
    for parameter in &block.parameters {
        match &parameter.kind {
            BlockParameterKind::Plain => {}
            BlockParameterKind::ForOf { item: i, expression: e } => {
                (item, expression) = (Some(i.slice(source)), Some(e.slice(source)));
            }
            BlockParameterKind::Track { expression: e } => track = Some(e.slice(source)),
            BlockParameterKind::Let(variables) => lets.extend(
                variables.iter().map(|v| json!([v.name.slice(source), v.value.slice(source)])),
            ),
            BlockParameterKind::As { alias: a } => alias = Some(a.slice(source)),
        }
    }
    json!([item, expression, track, lets, alias])
}

fn template_bindings(source: &str, bindings: &[TemplateBinding]) -> Value {
    let s = |span: Option<oxc_htmlish_parser::Span>| span.map(|span| span.slice(source));
    bindings
        .iter()
        .map(|binding| match *binding {
            TemplateBinding::Expression { key, expression } => json!(["e", s(key), s(expression)]),
            TemplateBinding::Let { name, value } => json!(["l", s(Some(name)), s(value)]),
            TemplateBinding::As { key, alias } => json!(["a", s(key), s(Some(alias))]),
        })
        .collect()
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
                AttributeKind::Angular(AngularAttribute::Interpolation(expressions)) => {
                    let expressions: Vec<&str> =
                        expressions.iter().map(|e| e.slice(source)).collect();
                    json!([name, value, expressions])
                }
                AttributeKind::Angular(AngularAttribute::Template(bindings))
                    if a.value.is_some() =>
                {
                    json!([
                        name,
                        value,
                        ["tb", bindings.as_ref().map(|b| template_bindings(source, b))]
                    ])
                }
                AttributeKind::Plain | AttributeKind::Angular(_) => json!([name, value]),
                AttributeKind::Comment => json!(["comment", name]),
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
                    let part =
                        |part: Option<oxc_htmlish_parser::Span>| part.map(|p| p.slice(source));
                    let v_for = d.for_expression.as_ref().map(|f| {
                        json!([part(f.value), part(f.key), part(f.index), f.source.slice(source)])
                    });
                    json!([name, value, dir, arg, dynamic, modifiers, v_for])
                }
            }
        })
        .collect();
    json!(["el", el.name.slice(source), flags, attrs, children(source, root, id)])
}
