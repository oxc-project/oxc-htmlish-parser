//! The tag table: the only HTML tree-construction knowledge the parser has.
//!
//! Names are matched as given; callers lowercase them where the dialect is case-insensitive.
//! Implicit closing follows `angular-html-parser` (`html_tags.ts`), not WHATWG.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Normal,
    /// `script` / `style`: no markup until the end tag.
    Raw,
    /// `textarea` / `title`: no markup until the end tag, but Vue `{{ }}` still splits.
    Escapable,
}

pub fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// `lowercase` is the lowercased name: content types are case-insensitive in every dialect.
pub fn content(lowercase: &str, in_svg: bool) -> Content {
    match lowercase {
        "script" | "style" => Content::Raw,
        "textarea" => Content::Escapable,
        "title" if !in_svg => Content::Escapable,
        _ => Content::Normal,
    }
}

pub fn ignores_first_lf(name: &str) -> bool {
    matches!(name, "pre" | "textarea" | "listing")
}

/// Whether an end tag of an ancestor may close this element implicitly.
pub fn closed_by_parent(name: &str) -> bool {
    matches!(
        name,
        "p" | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "li"
            | "dd"
            | "rb"
            | "rt"
            | "rtc"
            | "rp"
            | "optgroup"
            | "option"
    )
}

/// Whether opening `child` right inside `parent` closes `parent`.
pub fn closed_by_child(parent: &str, child: &str) -> bool {
    match parent {
        "p" => matches!(
            child,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "div"
                | "dl"
                | "fieldset"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hgroup"
                | "hr"
                | "main"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "section"
                | "table"
                | "ul"
        ),
        "thead" | "tbody" => matches!(child, "tbody" | "tfoot"),
        "tfoot" => child == "tbody",
        "tr" => child == "tr",
        "td" | "th" => matches!(child, "td" | "th"),
        "li" => child == "li",
        "dt" | "dd" => matches!(child, "dt" | "dd"),
        "rb" | "rt" | "rp" => matches!(child, "rb" | "rt" | "rtc" | "rp"),
        "rtc" => matches!(child, "rb" | "rtc" | "rp"),
        "optgroup" => child == "optgroup",
        "option" => matches!(child, "option" | "optgroup"),
        _ => false,
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
