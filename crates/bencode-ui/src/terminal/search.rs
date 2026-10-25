use alacritty_terminal::term::search::Match;

use super::view::Terminal;

/// A find's matches over the scrollback and the screen, and the one stepped to.
#[derive(Default)]
pub(crate) struct Search {
    all: Vec<Match>,
    current: Option<usize>,
}

impl Search {
    pub(crate) fn visible(&self) -> (&[Match], Option<usize>) {
        (&self.all, self.current)
    }
}

impl Terminal {}
