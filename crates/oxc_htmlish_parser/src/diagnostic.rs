//! Parse errors.
//!
//! Only truly broken structure is an error (as in Prettier); the parse stops at the first one.
//! What a dialect's own compiler accepts is never an error here
//! (unknown entities, `<DIV></div>`, `{{ a<b }}` in Vue).

use std::fmt::Display;

use crate::span::Span;

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
}

impl Diagnostic {
    pub(crate) fn new(kind: DiagnosticKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// The source exceeds the maximum supported size (4 GiB).
    SourceTooLong,
    UnterminatedComment,
    UnterminatedCdata,
    UnterminatedDocType,
    UnterminatedStartTag,
    UnterminatedEndTag,
    UnterminatedAttributeValue,
    UnterminatedInterpolation,
    /// A raw text element (`<script>`, `<style>`, a Vue SFC block, …) without its end tag.
    UnterminatedRawText,
    /// A `/` that no `>` follows in a start tag, or a quote in an attribute name.
    UnexpectedCharacterInTag,
    /// An end tag with no open element to close,
    /// or one that would close an element whose end tag cannot be omitted.
    UnexpectedEndTag,
    /// `</br>`.
    VoidEndTag,
    /// Angular: `@if (a)` without `{`, or parameters without `)`.
    IncompleteBlock,
    /// Angular: a `}` with no open block to close,
    /// or one that would close an element whose end tag cannot be omitted.
    UnexpectedBlockClose,
    /// Angular: a block without its `}`.
    UnclosedBlock,
}

impl Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self.kind {
            DiagnosticKind::SourceTooLong => "source exceeds the maximum supported size (4 GiB)",
            DiagnosticKind::UnterminatedComment => "unterminated comment",
            DiagnosticKind::UnterminatedCdata => "unterminated CDATA section",
            DiagnosticKind::UnterminatedDocType => "unterminated doctype",
            DiagnosticKind::UnterminatedStartTag => "unterminated start tag",
            DiagnosticKind::UnterminatedEndTag => "unterminated end tag",
            DiagnosticKind::UnterminatedAttributeValue => "unterminated attribute value",
            DiagnosticKind::UnterminatedInterpolation => "unterminated interpolation",
            DiagnosticKind::UnterminatedRawText => "missing end tag of raw text element",
            DiagnosticKind::UnexpectedCharacterInTag => "unexpected character in tag",
            DiagnosticKind::UnexpectedEndTag => "unexpected end tag",
            DiagnosticKind::VoidEndTag => "void elements do not have end tags",
            DiagnosticKind::IncompleteBlock => "incomplete block",
            DiagnosticKind::UnexpectedBlockClose => "unexpected block close",
            DiagnosticKind::UnclosedBlock => "unclosed block",
        };
        write!(f, "{message} at {}..{}", self.span.start, self.span.end)
    }
}
