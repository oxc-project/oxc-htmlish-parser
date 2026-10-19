#![expect(clippy::print_stdout)]
use oxc_htmlish_parser::{Allocator, Dialect, Parser};

fn main() {
    let path = std::env::args().nth(1).expect("usage: parse <file>");
    let source = std::fs::read_to_string(&path).expect("failed to read file");
    let dialect =
        if std::path::Path::new(&path).extension().is_some_and(|e| e.eq_ignore_ascii_case("vue")) {
            Dialect::Vue
        } else {
            Dialect::Html
        };
    let allocator = Allocator::default();
    match Parser::new(&allocator, &source, dialect).parse() {
        Ok(root) => println!("{root:#?}"),
        Err(diagnostic) => println!("Error: {diagnostic}"),
    }
}
