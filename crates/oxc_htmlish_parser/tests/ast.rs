//! AST snapshots: one compact tree per fixture, spans and facts included.
//!
//! `fixtures/<dialect>/*` parse as that dialect (`html`, `xhtml`, `vue`, `angular`).
//! A fixture that fails to parse snapshots its diagnostic.
//! Review with `cargo insta review` after an intentional AST change.

use std::{fmt::Write as _, fs, path::Path};

use insta::{Settings, assert_snapshot, glob};

use oxc_htmlish_parser::{
    Allocator, Dialect, Parser, Span,
    ast::{
        AngularAttribute, AngularCode, AngularExpression, Attribute, AttributeKind,
        BlockParameterKind, Content, Interpolation, NodeId, NodeKind, Quote, Root, TemplateBinding,
        Whitespace,
    },
};

mod check;

#[test]
fn ast_snapshots() {
    glob!("fixtures/*/*.{html,vue}", |path| {
        let source = fs::read_to_string(path).unwrap();
        let dialect = match path.parent().unwrap().file_name().unwrap().to_str().unwrap() {
            "html" => Dialect::Html,
            "xhtml" => Dialect::Xhtml,
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
            NodeKind::Interpolation(n) => format!(
                "Interpolation expression={}{}",
                q(Interpolation::expression(node.span)),
                n.angular
                    .as_deref()
                    .map_or_else(String::new, |e| angular_expression(self.source, e))
            ),
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
                let parameters: Vec<_> = b.parameters.iter().map(|p| q(p.span)).collect();
                let close = b.close.map_or("no_body".to_string(), |c| format!("close={}", q(c)));
                format!(
                    "Block {:?} name={} parameters={parameters:?} open={} {close}",
                    b.kind,
                    q(b.name),
                    q(b.open)
                )
            }
            NodeKind::Icu(n) => {
                format!("Icu expression={} type={}", q(n.expression), q(n.type_name))
            }
            NodeKind::IcuCase(n) => {
                format!("IcuCase value={} open={}", q(n.value), n.open)
            }
            NodeKind::LetDeclaration(n) => {
                format!("LetDeclaration name={} value={}", q(n.name), q(n.value))
            }
        };
        self.line(&head);
        if let NodeKind::Block(block) = &node.kind {
            self.depth += 1;
            for parameter in &block.parameters {
                self.block_parameter(&parameter.kind);
            }
            self.depth -= 1;
        }
        if let NodeKind::Element(el) = &node.kind {
            self.depth += 1;
            for attribute in &el.attributes {
                self.attribute(attribute);
            }
            self.depth -= 1;
        }
        self.children(id);
    }

    fn block_parameter(&mut self, kind: &BlockParameterKind<'_>) {
        let q = |span| quoted(self.source, span);
        let line = match kind {
            BlockParameterKind::Plain => return,
            BlockParameterKind::ForOf { item, expression } => {
                format!("% for item={} of={}", q(*item), q(*expression))
            }
            BlockParameterKind::Track { expression } => format!("% track {}", q(*expression)),
            BlockParameterKind::Let(variables) => {
                let variables: Vec<_> =
                    variables.iter().map(|v| format!("{} = {}", q(v.name), q(v.value))).collect();
                format!("% let {}", variables.join(", "))
            }
            BlockParameterKind::As { alias } => format!("% as {}", q(*alias)),
        };
        self.line(&line);
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
        match &attribute.kind {
            AttributeKind::Plain => {}
            AttributeKind::Comment => line.push_str(" comment"),
            AttributeKind::Angular(kind) => {
                let name = match kind {
                    AngularAttribute::Property(_) => "Property",
                    AngularAttribute::Event(_) => "Event",
                    AngularAttribute::TwoWay(_) => "TwoWay",
                    AngularAttribute::Template(_) => "Template",
                    AngularAttribute::I18n => "I18n",
                    AngularAttribute::Interpolation(_) => "Interpolation",
                };
                let target = kind.target(attribute.name).slice(self.source);
                write!(line, " {name} target={target:?}").unwrap();
                if let AngularAttribute::Interpolation(expressions) = kind {
                    let texts: Vec<_> =
                        expressions.iter().map(|e| e.expression.slice(self.source)).collect();
                    write!(line, " {texts:?}").unwrap();
                    for e in expressions.iter() {
                        if let Some(angular) = &e.angular {
                            line.push_str(&angular_expression(self.source, angular));
                        }
                    }
                }
                if let AngularAttribute::Property(Some(expression))
                | AngularAttribute::TwoWay(Some(expression)) = kind
                {
                    line.push_str(&angular_expression(self.source, expression));
                }
                if let AngularAttribute::Event(Some(action)) = kind {
                    let statements: Vec<_> = action
                        .statements
                        .iter()
                        .map(|s| {
                            format!(
                                "{}{:?}",
                                if s.conditional { "?" } else { "" },
                                s.span.slice(self.source)
                            )
                        })
                        .collect();
                    write!(line, " statements=[{}]", statements.join(", ")).unwrap();
                    if let Some(comment) = action.comment {
                        write!(line, " comment={:?}", comment.slice(self.source)).unwrap();
                    }
                }
                if let AngularAttribute::Template(Some(bindings)) = kind {
                    let s = |span: Option<Span>| span.map(|span| span.slice(self.source));
                    for binding in bindings.iter() {
                        match *binding {
                            TemplateBinding::Expression { key, expression } => {
                                write!(line, " [{:?}: {:?}]", s(key), s(expression))
                            }
                            TemplateBinding::Let { name, value } => {
                                write!(line, " [let {:?} = {:?}]", s(Some(name)), s(value))
                            }
                            TemplateBinding::As { key, alias } => {
                                write!(line, " [{:?} as {:?}]", s(key), s(Some(alias)))
                            }
                        }
                        .unwrap();
                    }
                }
            }
            AttributeKind::Vue(directive) => {
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
                if let Some(v_for) = &directive.for_expression {
                    let part = |part: Option<Span>| part.map(|part| part.slice(self.source));
                    write!(
                        line,
                        " for=({:?} {:?} {:?} {:?} {} {:?})",
                        v_for.aliases.slice(self.source),
                        part(v_for.value),
                        part(v_for.key),
                        part(v_for.index),
                        v_for.operator.slice(self.source),
                        v_for.source.slice(self.source),
                    )
                    .unwrap();
                }
            }
        }
        self.line(&line);
    }
}

/// `[a..b] "slice"`: the slice makes offsets reviewable.
fn quoted(source: &str, span: Span) -> String {
    format!("[{}..{}] {:?}", span.start, span.end, span.slice(source))
}

/// ` code="a" | p["x", "y"] comment="// c"`
fn angular_expression(source: &str, e: &AngularExpression<'_>) -> String {
    let piece = |code: &AngularCode| {
        let conditional = if code.conditional { "?" } else { "" };
        format!("{conditional}{:?}", code.span.slice(source))
    };
    let mut out = format!(" code={}", piece(&e.code));
    for pipe in &e.pipes {
        let arguments: Vec<_> = pipe.arguments.iter().map(piece).collect();
        write!(out, " | {}[{}]", pipe.name.slice(source), arguments.join(", ")).unwrap();
    }
    if let Some(comment) = e.comment {
        write!(out, " comment={:?}", comment.slice(source)).unwrap();
    }
    out
}
