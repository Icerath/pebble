use std::{
    fmt,
    ops::{Index, Range},
};

#[derive(Clone, Copy)]
pub struct Span {
    start: u32,
    end: u32,
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.into_range().fmt(f)
    }
}

impl Span {
    pub fn shrink(self, n: u32) -> Self {
        (self.start + n..self.end - n).into()
    }
    pub const fn into_range(self) -> Range<u32> {
        self.start..self.end
    }
    pub const fn into_range_usize(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl From<Range<u32>> for Span {
    fn from(Range { start, end }: Range<u32>) -> Self {
        Self { start, end }
    }
}

impl Index<Span> for str {
    type Output = Self;
    fn index(&self, index: Span) -> &Self::Output {
        &self[index.into_range_usize()]
    }
}

impl From<Span> for miette::SourceSpan {
    fn from(span: Span) -> Self {
        Self::from(span.into_range_usize())
    }
}
