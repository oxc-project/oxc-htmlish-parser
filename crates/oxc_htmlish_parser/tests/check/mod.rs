//! The AST invariants a formatter relies on, shared by `tests/ast.rs` and `tasks/sig` (the differential fuzz):
//! links are consistent (`next` chains end at `last_child`); siblings are ordered and disjoint,
//! children and attributes lie inside their parent.

use oxc_htmlish_parser::{
    Span,
    ast::{NodeId, NodeKind, Root},
};

/// The first broken invariant under `id`.
pub fn check(root: &Root<'_>, id: NodeId) -> Result<(), String> {
    let parent = &root[id];
    let inner = match &parent.kind {
        NodeKind::Element(el) => {
            for attribute in &el.attributes {
                let span = attribute.span;
                if !(el.start_tag.start < span.start && span.end <= el.start_tag.end) {
                    return Err(format!("attribute {span:?} outside {:?}", el.start_tag));
                }
            }
            Span::new(el.start_tag.end, el.end_tag.map_or(parent.span.end, |tag| tag.span.start))
        }
        NodeKind::ConditionalComment(n) => n.content,
        NodeKind::Block(b) => Span::new(b.open.end, b.close.map_or(parent.span.end, |c| c.start)),
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
