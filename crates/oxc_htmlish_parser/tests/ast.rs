//! AST snapshots: one compact tree per fixture, spans and facts included.
//!
//! `fixtures/html/*.html` parse as HTML, `fixtures/vue/*.vue` as Vue.
//! A fixture that fails to parse snapshots its diagnostic.
//! Review with `cargo insta review` after an intentional AST change.

use std::{fmt::Write as _, fs, path::Path};

use insta::{Settings, assert_snapshot, glob};
use oxc_htmlish_parser::{
    Allocator, Dialect, Parser, Span,
    ast::{Attribute, AttributeKind, Node, Quote, Whitespace},
};

struct Printer<'s> {
    source: &'s str,
    out: String,
    depth: usize,
}

impl Printer<'_> {
    /// `[a..b] "slice"`: the slice makes offsets reviewable.
    fn quoted(&self, span: Span) -> String {
        format!("[{}..{}] {:?}", span.start, span.end, span.slice(self.source))
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(&"  ".repeat(self.depth));
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn nodes(&mut self, nodes: &[Node<'_>]) {
        self.depth += 1;
        for node in nodes {
            self.node(node);
        }
        self.depth -= 1;
    }

    fn node(&mut self, node: &Node<'_>) {
        match node {
            Node::Element(el) => {
                let mut head = format!("Element {} {:?}", self.quoted(el.name), el.namespace);
                if el.self_closing {
                    head.push_str(" self_closing");
                }
                if el.leading_lf {
                    head.push_str(" leading_lf");
                }
                match el.end_tag {
                    Some(tag) => write!(head, " end_tag={}", self.quoted(tag)).unwrap(),
                    None => write!(head, " no_end_tag span_end={}", el.span.end).unwrap(),
                }
                self.line(&head);
                self.depth += 1;
                for attribute in &el.attributes {
                    self.attribute(attribute);
                }
                self.depth -= 1;
                self.nodes(&el.children);
            }
            Node::Text(text) => {
                let ws = |w: Whitespace| match w {
                    Whitespace::None => "-",
                    Whitespace::Space => "sp",
                    Whitespace::Newline => "nl",
                };
                let content = if text.is_whitespace_only() {
                    String::new()
                } else {
                    format!(" content={}", self.quoted(text.content))
                };
                let line = format!(
                    "Text {} ({}|{}){content}",
                    self.quoted(text.span),
                    ws(text.leading),
                    ws(text.trailing)
                );
                self.line(&line);
            }
            Node::Interpolation(n) => {
                let line = format!("Interpolation expression={}", self.quoted(n.expression));
                self.line(&line);
            }
            Node::Comment(n) => {
                let bogus = if n.bogus { " bogus" } else { "" };
                let line = format!("Comment{bogus} content={}", self.quoted(n.content));
                self.line(&line);
            }
            Node::ConditionalComment(n) => {
                let line = format!(
                    "ConditionalComment condition={} content={}",
                    self.quoted(n.condition),
                    self.quoted(n.content)
                );
                self.line(&line);
                if let Some(children) = &n.children {
                    self.nodes(children);
                } else {
                    self.depth += 1;
                    self.line("(unparsed)");
                    self.depth -= 1;
                }
            }
            Node::ConditionalStart(n) => {
                let line = format!("ConditionalStart condition={}", self.quoted(n.condition));
                self.line(&line);
            }
            Node::ConditionalEnd(n) => {
                let line = format!("ConditionalEnd {}", self.quoted(n.span));
                self.line(&line);
            }
            Node::DocType(n) => {
                let line = format!("DocType value={}", self.quoted(n.value));
                self.line(&line);
            }
            Node::Cdata(n) => {
                let line = format!("Cdata {}", self.quoted(n.span));
                self.line(&line);
            }
        }
    }

    fn attribute(&mut self, attribute: &Attribute<'_>) {
        let mut line = format!("@{}", self.quoted(attribute.name));
        if let Some(value) = attribute.value {
            let quote = match value.quote {
                Quote::None => "",
                Quote::Single => "'",
                Quote::Double => "\"",
            };
            write!(line, " ={quote} {}", self.quoted(value.span)).unwrap();
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

#[test]
fn ast_snapshots() {
    glob!("fixtures/**/*.{html,vue}", |path| {
        let source = fs::read_to_string(path).unwrap();
        let dialect = match path.extension().and_then(|e| e.to_str()) {
            Some("vue") => Dialect::Vue,
            _ => Dialect::Html,
        };
        let allocator = Allocator::default();
        let mut printer = Printer { source: &source, out: String::new(), depth: 0 };
        match Parser::new(&allocator, &source, dialect).parse() {
            Ok(root) => {
                check_spans(&root.children, root.span);
                printer.line(&format!("Root sfc={}", root.sfc));
                printer.nodes(&root.children);
            }
            Err(diagnostic) => printer.line(&format!("Error: {diagnostic}")),
        }
        snapshot(path, &printer.out);
    });
}

/// Siblings are ordered and disjoint, children and attributes lie inside their parent.
fn check_spans(nodes: &[Node<'_>], parent: Span) {
    let mut at = parent.start;
    for node in nodes {
        let span = node.span();
        assert!(
            at <= span.start && span.start < span.end && span.end <= parent.end,
            "{span:?} in {parent:?}"
        );
        at = span.end;
        match node {
            Node::Element(el) => {
                let inner =
                    Span::new(el.start_tag.end, el.end_tag.map_or(el.span.end, |tag| tag.start));
                for attribute in &el.attributes {
                    assert!(
                        el.start_tag.start < attribute.span.start
                            && attribute.span.end <= el.start_tag.end
                    );
                }
                check_spans(&el.children, inner);
            }
            Node::ConditionalComment(n) => {
                if let Some(children) = &n.children {
                    check_spans(children, n.content);
                }
            }
            _ => {}
        }
    }
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
