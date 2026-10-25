use crate::typography::format::Separators;

/// Which way a line of text runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Direction {}

/// A language as it writes numbers, dates and lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locale {
    pub tag: &'static str,
    pub name: &'static str,
    pub separators: Separators,
    /// A numeric date, as `strftime` reads it.
    pub date: &'static str,
    pub direction: Direction,
}

impl Locale {}
