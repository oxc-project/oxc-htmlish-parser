//! AST snapshots: one compact tree per fixture, spans and facts included.
//!
//! `fixtures/<dialect>/*` parse as that dialect (`html`, `vue`, `angular`).
//! A fixture that fails to parse snapshots its diagnostic.
//! Review with `cargo insta review` after an intentional AST change.

use std::{fmt::Write as _, fs, path::Path};

use insta::{Settings, assert_snapshot, glob};

use oxc_htmlish_parser::{
    Allocator, Dialect, Parser, Span,
    ast::{Attribute, AttributeKind, Content, NodeId, NodeKind, Quote, Root, Whitespace},
};

mod check;

#[test]
fn ast_snapshots() {
    glob!("fixtures/*/*.{html,vue}", |path| {
        let source = fs::read_to_string(path).unwrap();
        let dialect = match path.parent().unwrap().file_name().unwrap().to_str().unwrap() {
            "html" => Dialect::Html,
            "vue" => Dialect::Vue,
            "angular" => Dialect::Angular,
            dir => panic!("no dialect for fixtures/{dir}"),
        };
        let allocator = Allocator::default();
        let out = match Parser::new(&allocator, &source, dialect).parse() {
            Ok(root) => {
                check::check(&root, NodeId::DOCUMENT).unwrap();
                let mut printer =
                    Printer { source: &source, root: &root, out: String::new(), depth: 0 };
                printer.line(&format!("Root sfc={}", root.sfc));
                printer.children(NodeId::DOCUMENT);
                printer.out
            }
            Err(diagnostic) => format!("Error: {diagnostic}\n"),
        };
        snapshot(path, &out);
    });
}

fn snapshot(path: &Path, out: &str) {
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let mut settings = Settings::clone_current();
    settings.set_snapshot_path(path.parent().unwrap());
    settings.remove_snapshot_suffix();
    settings.set_prepend_module_to_snapshot(false);
    settings.remove_input_file();
    settings.set_omit_expression(true);
    settings.bind(|| {
        assert_snapshot!(file_name.to_string(), out);
    });
}

// ---

struct Printer<'s, 'a> {
    source: &'s str,
    root: &'s Root<'a>,
    out: String,
    depth: usize,
}

impl Printer<'_, '_> {
    fn line(&mut self, text: &str) {
        self.out.push_str(&"  ".repeat(self.depth));
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn children(&mut self, id: NodeId) {
        self.depth += 1;
        for (child, _) in self.root.children(id) {
            self.node(child);
        }
        self.depth -= 1;
    }

    fn node(&mut self, id: NodeId) {
        let node = &self.root[id];
        let q = |span| quoted(self.source, span);
        let head = match &node.kind {
            NodeKind::Document => unreachable!(),
            NodeKind::Element(el) => {
                let mut head = format!("Element {} {:?}", q(el.name), el.namespace);
                if el.key != el.name.slice(self.source) {
                    write!(head, " key={:?}", el.key).unwrap();
                }
                if el.void {
                    head.push_str(" void");
                }
                match el.content {
                    Content::Normal => {}
                    Content::Raw => head.push_str(" raw"),
                    Content::Escapable => head.push_str(" escapable"),
                }
                if el.self_closing {
                    head.push_str(" self_closing");
                }
                if el.ignores_first_lf {
                    head.push_str(if el.leading_lf { " leading_lf" } else { " ignores_first_lf" });
                }
                match el.end_tag {
                    Some(tag) => {
                        write!(head, " end_tag={}", q(tag.span)).unwrap();
                        if tag.name.slice(self.source) != el.name.slice(self.source) {
                            write!(head, " end_name={}", q(tag.name)).unwrap();
                        }
                        if let Some(ignored) = tag.ignored {
                            write!(head, " ignored={}", q(ignored)).unwrap();
                        }
                    }
                    None => write!(head, " no_end_tag span_end={}", node.span.end).unwrap(),
                }
                head
            }
            NodeKind::Text(text) => {
                let ws = |w: Whitespace| match w {
                    Whitespace::None => "-",
                    Whitespace::Space => "sp",
                    Whitespace::Newline => "nl",
                };
                let content = if text.is_whitespace_only() {
                    String::new()
                } else {
                    format!(" content={}", q(text.content))
                };
                format!(
                    "Text {} ({}|{}){content}",
                    q(node.span),
                    ws(text.leading),
                    ws(text.trailing)
                )
            }
            NodeKind::Interpolation(n) => format!("Interpolation expression={}", q(n.expression)),
            NodeKind::Comment(n) => {
                let bogus = if n.bogus { " bogus" } else { "" };
                format!("Comment{bogus} content={}", q(n.content))
            }
            NodeKind::ConditionalComment(n) => {
                let unparsed = if n.parsed { "" } else { " unparsed" };
                format!(
                    "ConditionalComment condition={} content={}{unparsed}",
                    q(n.condition),
                    q(n.content)
                )
            }
            NodeKind::ConditionalStart(n) => {
                format!("ConditionalStart condition={}", q(n.condition))
            }
            NodeKind::ConditionalEnd => format!("ConditionalEnd {}", q(node.span)),
            NodeKind::DocType(n) => format!("DocType value={}", q(n.value)),
            NodeKind::Cdata => format!("Cdata {}", q(node.span)),
            NodeKind::EndTag(tag) => format!("EndTag name={}", q(tag.name)),
            NodeKind::Block(b) => {
                let parameters: Vec<_> = b.parameters.iter().map(|p| q(*p)).collect();
                let close = b.close.map_or("no_body".to_string(), |c| format!("close={}", q(c)));
                format!(
                    "Block name={} parameters={parameters:?} open={} {close}",
                    q(b.name),
                    q(b.open)
                )
            }
        };
        self.line(&head);
        if let NodeKind::Element(el) = &node.kind {
            self.depth += 1;
            for attribute in &el.attributes {
                self.attribute(attribute);
            }
            self.depth -= 1;
        }
        self.children(id);
    }

    fn attribute(&mut self, attribute: &Attribute<'_>) {
        let mut line = format!("@{}", quoted(self.source, attribute.name));
        if let Some(value) = attribute.value {
            let quote = match value.quote {
                Quote::None => "",
                Quote::Single => "'",
                Quote::Double => "\"",
            };
            write!(line, " ={quote} {}", quoted(self.source, value.span)).unwrap();
        }
        if let AttributeKind::Vue(directive) = &attribute.kind {
            write!(line, " vue name={:?}", directive.name.slice(self.source)).unwrap();
            if let Some(argument) = directive.argument {
                let dynamic = if argument.dynamic { " dynamic" } else { "" };
                write!(line, " arg={:?}{dynamic}", argument.span.slice(self.source)).unwrap();
            }
            if !directive.modifiers.is_empty() {
                let modifiers: Vec<_> =
                    directive.modifiers.iter().map(|m| m.slice(self.source)).collect();
                write!(line, " modifiers={modifiers:?}").unwrap();
            }
        }
        self.line(&line);
    }
}

/// `[a..b] "slice"`: the slice makes offsets reviewable.
fn quoted(source: &str, span: Span) -> String {
    format!("[{}..{}] {:?}", span.start, span.end, span.slice(source))
}
