/// A span from `usize` offsets inside the source, which `Parser::parse` checked to fit in `u32`.
#[expect(clippy::cast_possible_truncation)]
pub fn span(start: usize, end: usize) -> Span {
    Span::new(start as u32, end as u32)
}

/// A byte range of the source, 0-based.
///
/// Offsets are `u32` (matching oxc convention);
/// sources larger than 4 GiB are rejected by the parser up front.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Span {
    /// Start offset. (Inclusive)
    pub start: u32,
    /// End offset. (Exclusive)
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// A zero-width span at the given offset.
    pub fn empty(at: u32) -> Self {
        Self { start: at, end: at }
    }

    /// The source text this span covers.
    pub fn slice(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
    }

    /// Whether the span covers no characters.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}
