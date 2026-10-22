//! The tag table: the only HTML tree-construction knowledge the parser has.
//!
//! Lookups take lowercase names; the parser's `Builder::tag` applies each dialect's case and namespace policy on top.
//! Implicit closing follows WHATWG's optional end tags in HTML, where browsers build the page from the source,
//! and `angular-html-parser` (`html_tags.ts`, Angular's own compiler's) otherwise.

use crate::ast::{Content, Namespace};

/// What the table says about one element.
#[derive(Clone, Copy, Default)]
pub struct Tag {
    pub void: bool,
    pub content: Content,
    /// HTML drops a line break right after the start tag.
    pub ignores_first_lf: bool,
    /// An ancestor's end tag may close it implicitly.
    pub closed_by_parent: bool,
    /// Start tags that close it when opened right inside it.
    pub closed_by: &'static [&'static str],
}

/// `whatwg`: WHATWG's optional end tags; otherwise `angular-html-parser`'s.
pub fn html(lowercase: &str, whatwg: bool) -> Tag {
    // WHATWG adds `details`, `dialog`, `figcaption`, `figure`, `menu` and `search`,
    // which would make a written `</p>` after them an error (`<p><figure></figure></p>`).
    const P_CLOSERS: &[&str] = &[
        "address",
        "article",
        "aside",
        "blockquote",
        "div",
        "dl",
        "fieldset",
        "footer",
        "form",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hgroup",
        "hr",
        "main",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "table",
        "ul",
    ];
    const RUBY: &[&str] = &["rb", "rt", "rtc", "rp"];
    let closes =
        |closed_by_parent, closed_by| Tag { closed_by_parent, closed_by, ..Tag::default() };
    match lowercase {
        "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta"
        | "param" | "source" | "track" | "wbr" => Tag { void: true, ..Tag::default() },
        "script" | "style" => Tag { content: Content::Raw, ..Tag::default() },
        "textarea" => Tag { content: Content::Escapable, ignores_first_lf: true, ..Tag::default() },
        "title" => Tag { content: Content::Escapable, ..Tag::default() },
        "pre" | "listing" => Tag { ignores_first_lf: true, ..Tag::default() },
        "p" => closes(true, P_CLOSERS),
        "thead" => closes(false, &["tbody", "tfoot"]),
        "tbody" => closes(true, &["tbody", "tfoot"]),
        "tfoot" => closes(true, &["tbody"]),
        // Without the `tbody` WHATWG would insert, a row's group ends with it.
        "tr" if whatwg => closes(true, &["tr", "tbody", "tfoot"]),
        "tr" => closes(true, &["tr"]),
        "td" | "th" => closes(true, &["td", "th"]),
        "li" => closes(true, &["li"]),
        "dt" => closes(false, &["dt", "dd"]),
        "dd" => closes(true, &["dt", "dd"]),
        "rb" | "rt" | "rp" => closes(true, RUBY),
        "rtc" => closes(true, &["rb", "rtc", "rp"]),
        "optgroup" if whatwg => closes(true, &["optgroup", "hr"]),
        "optgroup" => closes(true, &["optgroup"]),
        "option" if whatwg => closes(true, &["option", "optgroup", "hr"]),
        "option" => closes(true, &["option", "optgroup"]),
        // Not at what it cannot hold: pages put elements there before an explicit `</head>`.
        "head" if whatwg => closes(true, &["body"]),
        // Closed by what may follow them in a table.
        "caption" | "colgroup" if whatwg => {
            closes(true, &["colgroup", "thead", "tbody", "tfoot", "tr"])
        }
        "body" if whatwg => closes(true, &[]),
        _ => Tag::default(),
    }
}

/// The namespace an element named `key` opens in, given the one its parent's children inherit.
pub fn namespace(key: &str, inherited: Namespace) -> Namespace {
    match key {
        "svg" => Namespace::Svg,
        "math" => Namespace::MathMl,
        _ => inherited,
    }
}

/// The namespace an element's children inherit: `foreignObject` holds HTML again.
pub fn children_namespace(name: &str, namespace: Namespace) -> Namespace {
    if namespace == Namespace::Svg && name.eq_ignore_ascii_case("foreignObject") {
        Namespace::Html
    } else {
        namespace
    }
}

/// Lowercases `name` into `buf`.
/// Names longer than every table entry come back as `""`, which matches nothing.
pub fn lowercase<'b>(name: &str, buf: &'b mut [u8; 16]) -> &'b str {
    let Some(buf) = buf.get_mut(..name.len()) else { return "" };
    buf.copy_from_slice(name.as_bytes());
    buf.make_ascii_lowercase();
    // ASCII lowercasing keeps the bytes valid UTF-8.
    std::str::from_utf8(buf).unwrap_or("")
}
