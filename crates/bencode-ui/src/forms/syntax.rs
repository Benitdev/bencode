//! How one language's code reads, by file name.

/// A language's comments, keywords and constants.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Syntax {
    /// The mark that opens a line comment.
    pub comment: &'static str,
    /// The mark counts only where a word starts (Shell).
    pub word_comment: bool,
    pub keywords: &'static [&'static str],
    pub constants: &'static [&'static str],
}

impl Syntax {}
