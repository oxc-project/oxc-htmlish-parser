//! The AST invariants a formatter relies on, shared by `tests/ast.rs` and `tasks/sig` (the differential fuzz):
//! links are consistent (`next` chains end at `last_child`);
//! siblings are ordered and disjoint, inside their parent, and so are attributes inside the start tag;
//! an attribute holds its name, then its value; a directive's parts are in the name, in order, and `v-for`'s in the value;
//! a block's parameters are in its start, in order, and their parts in each; a microsyntax's parts are in the value.

use oxc_htmlish_parser::{
    Span,
    ast::{
        AngularAttribute, AttributeKind, BlockParameterKind, NodeId, NodeKind, Root,
        TemplateBinding,
    },
};

/// The first broken invariant under `id`.
pub fn check(root: &Root<'_>, id: NodeId) -> Result<(), String> {
    let parent = &root[id];
    let inner = match &parent.kind {
        NodeKind::Element(el) => {
            let mut at = el.name.end;
            for attribute in &el.attributes {
                let (span, name) = (attribute.span(), attribute.name);
                if !(at <= span.start && span.end <= el.start_tag.end) {
                    return Err(format!("attribute {span:?} after {at} in {:?}", el.start_tag));
                }
                at = span.end;
                let value = attribute.value.map_or(Span::empty(name.end), |v| v.span);
                if ![span.start, name.start, name.end, value.start, value.end, span.end].is_sorted()
                {
                    return Err(format!("name {name:?} and value {value:?} in {span:?}"));
                }
                // A new kind lists its own parts here
                let parts: Vec<Span> = match &attribute.kind {
                    AttributeKind::Plain | AttributeKind::Comment(_) => Vec::new(),
                    AttributeKind::Angular(kind) => {
                        if let AngularAttribute::Template(Some(bindings)) = kind {
                            let parts = bindings.iter().flat_map(|binding| match binding {
                                TemplateBinding::Expression { key, expression, .. } => {
                                    [*key, *expression]
                                }
                                TemplateBinding::Let { name, value } => [Some(*name), *value],
                                TemplateBinding::As { key, alias } => [*key, Some(*alias)],
                            });
                            if bindings.is_empty()
                                || !parts
                                    .flatten()
                                    .all(|part| value.start <= part.start && part.end <= value.end)
                            {
                                return Err(format!("template bindings {bindings:?} in {value:?}"));
                            }
                        }
                        if let AngularAttribute::Interpolation(expressions) = kind
                            && !expressions
                                .iter()
                                .map(|e| e.expression)
                                .all(|e| value.start + 2 <= e.start && e.end + 2 <= value.end)
                        {
                            return Err(format!("interpolations {expressions:?} in {value:?}"));
                        }
                        vec![kind.target(name)]
                    }
                    AttributeKind::Vue(directive) => {
                        if let Some(v_for) = &directive.for_expression {
                            let aliases =
                                [v_for.value, v_for.key, v_for.index].into_iter().flatten();
                            let bounds = [value.start, v_for.aliases.start]
                                .into_iter()
                                .chain(aliases.flat_map(|part| [part.start, part.end]))
                                .chain([
                                    v_for.aliases.end,
                                    v_for.operator.start,
                                    v_for.operator.end,
                                ])
                                .chain([v_for.source.start, v_for.source.end, value.end]);
                            if !bounds.is_sorted() {
                                return Err(format!("v-for parts {v_for:?} in {value:?}"));
                            }
                        }
                        std::iter::once(directive.name)
                            .chain(directive.argument.map(|argument| argument.span))
                            .chain(directive.modifiers.iter().copied())
                            .collect()
                    }
                };
                let bounds = parts.iter().flat_map(|part| [part.start, part.end]);
                if !std::iter::once(name.start).chain(bounds).chain([name.end]).is_sorted() {
                    return Err(format!("directive parts {parts:?} in {name:?}"));
                }
            }
            Span::new(el.start_tag.end, el.end_tag.map_or(parent.span.end, |tag| tag.span.start))
        }
        NodeKind::ConditionalComment(n) => n.content,
        NodeKind::Block(b) => {
            let mut at = b.open.start;
            for parameter in &b.parameters {
                let span = parameter.span;
                let parts: Vec<Span> = match &parameter.kind {
                    BlockParameterKind::Plain => vec![],
                    BlockParameterKind::ForOf { item, expression } => vec![*item, *expression],
                    BlockParameterKind::Track { expression } => vec![*expression],
                    BlockParameterKind::Let(variables) => {
                        variables.iter().flat_map(|v| [v.name, v.value]).collect()
                    }
                    BlockParameterKind::As { alias } => vec![*alias],
                };
                let bounds = parts.iter().flat_map(|part| [part.start, part.end]);
                let bounds =
                    [at, span.start].into_iter().chain(bounds).chain([span.end, b.open.end]);
                if !bounds.is_sorted() {
                    return Err(format!(
                        "block parameter {parameter:?} after {at} in {:?}",
                        b.open
                    ));
                }
                at = span.end;
            }
            Span::new(b.open.end, b.close.map_or(parent.span.end, |c| c.start))
        }
        NodeKind::IcuCase(c) => Span::new(c.open + 1, parent.span.end - 1),
        _ => parent.span,
    };
    let mut at = inner.start;
    let mut prev = None;
    for (child, node) in root.children(id) {
        if node.parent != Some(id) {
            return Err(format!("{child:?} links to parent {:?}, not {id:?}", node.parent));
        }
        let span = node.span;
        if !(at <= span.start && span.start < span.end && span.end <= inner.end) {
            return Err(format!("{span:?} after {at} in {inner:?}"));
        }
        at = span.end;
        prev = Some(child);
        check(root, child)?;
    }
    if parent.last_child != prev {
        return Err(format!("{id:?} ends at {:?}, its last child is {prev:?}", parent.last_child));
    }
    Ok(())
}
