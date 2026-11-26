use std::ops::Range;

use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions,
    term::{Term, cell::Cell, cell::Flags, color::Colors, search::Match},
    vte::ansi::CursorShape,
};
use gpui::Hsla;

use super::colors::Ink;

/// How a run of cells paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Style {
    pub fg: Hsla,
    /// None where the terminal's ground shows.
    pub bg: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// One screen line: a char per cell, and style runs over its bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Row {
    pub text: String,
    pub runs: Vec<(usize, Style)>,
    /// Cells with combining marks, painted on their own at their column: the column, the cluster and its style.
    pub clusters: Vec<(usize, String, Style)>,
}

/// Why cells are lit: selected, or a find match, the current one stronger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lit {
    Selected,
    Found,
    Current,
}

/// A terminal's screen as it paints.
#[derive(Clone, Debug, Default)]
pub(crate) struct Frame {
    pub rows: Vec<Row>,
    /// Lit cells: the row, its columns, and why.
    pub lit: Vec<(usize, Range<usize>, Lit)>,
    /// The cursor's row, column and shape, when it shows.
    pub cursor: Option<(usize, usize, CursorShape)>,
}

impl Row {
    /// Each cell's text, a combining cluster whole.
    pub(crate) fn cells(&self) -> Vec<String> {
        let mut cells: Vec<String> = self.text.chars().map(String::from).collect();
        for (column, cluster, _) in &self.clusters {
            cells[*column] = cluster.clone();
        }
        cells
    }
}

impl Frame {}

/// The screen as it shows now, scrolled back or not, with `found` lit and match `current` stronger.
pub(crate) fn frame<T: EventListener>(
    term: &Term<T>,
    ink: &Ink,
    found: &[Match],
    current: Option<usize>,
) -> Frame {
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let mut rows = vec![Row::default(); term.screen_lines()];
    let mut lit: Vec<(usize, Range<usize>, Lit)> = Vec::new();
    let (top, bottom) = (-offset, rows.len() as i32 - 1 - offset);
    let near: Vec<(usize, &Match)> = found
        .iter()
        .enumerate()
        .filter(|(_, hit)| hit.end().line.0 >= top && hit.start().line.0 <= bottom)
        .collect();
    for indexed in content.display_iter {
        let (point, cell) = (indexed.point, indexed.cell);
        let row = (point.line.0 + offset) as usize;
        // A concealed cell still occupies its column and paints its background.
        let blank = cell
            .flags
            .intersects(Flags::HIDDEN | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
        let look = style(cell, ink, content.colors);
        let cluster = cell
            .zerowidth()
            .filter(|marks| !blank && !marks.is_empty())
            .map(|marks| {
                std::iter::once(cell.c)
                    .chain(marks.iter().copied())
                    .collect::<String>()
            });
        push(
            &mut rows[row],
            if blank || cluster.is_some() {
                ' '
            } else {
                cell.c
            },
            look,
        );
        if let Some(cluster) = cluster {
            rows[row].clusters.push((point.column.0, cluster, look));
        }
        let why = if content.selection.is_some_and(|range| range.contains(point)) {
            Some(Lit::Selected)
        } else {
            near.iter()
                .find(|(_, hit)| hit.contains(&point))
                .map(|(ix, _)| {
                    if Some(*ix) == current {
                        Lit::Current
                    } else {
                        Lit::Found
                    }
                })
        };
        if let Some(why) = why {
            light(&mut lit, row, point.column.0, why);
        }
    }
    let at = content.cursor.point.line.0 + offset;
    let shows = content.cursor.shape != CursorShape::Hidden && (0..rows.len() as i32).contains(&at);
    let cursor = shows.then_some((
        at as usize,
        content.cursor.point.column.0,
        content.cursor.shape,
    ));
    Frame { rows, lit, cursor }
}

fn push(row: &mut Row, ch: char, style: Style) {
    let len = ch.len_utf8();
    row.text.push(ch);
    match row.runs.last_mut() {
        Some((bytes, last)) if *last == style => *bytes += len,
        _ => row.runs.push((len, style)),
    }
}

fn light(lit: &mut Vec<(usize, Range<usize>, Lit)>, row: usize, column: usize, why: Lit) {
    match lit.last_mut() {
        Some((at, columns, last)) if *at == row && columns.end == column && *last == why => {
            columns.end += 1
        }
        _ => lit.push((row, column..column + 1, why)),
    }
}

fn style(cell: &Cell, ink: &Ink, set: &Colors) -> Style {
    let flags = cell.flags;
    let (mut fg, mut bg) = (ink.resolve(cell.fg, set), ink.resolve(cell.bg, set));
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(Flags::DIM) {
        fg = fg.opacity(0.7);
    }
    let visible = !flags.contains(Flags::HIDDEN);
    Style {
        fg,
        bg: (bg != ink.bg).then_some(bg),
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline: visible && flags.intersects(Flags::ALL_UNDERLINES),
        strike: visible && flags.contains(Flags::STRIKEOUT),
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::{
        event::VoidListener,
        term::{Config, test::TermSize},
        vte::ansi::Processor,
    };
    use gpui::{black, white};

    use super::*;

    fn replayed(bytes: &[u8]) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &TermSize::new(12, 3), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, bytes);
        term
    }

    fn ink() -> Ink {
        let mut ansi = [black(); 16];
        ansi[1] = gpui::red();
        Ink {
            ansi,
            fg: black(),
            bg: white(),
        }
    }

    #[test]
    fn a_wide_char_keeps_the_columns_after_it() {
        let term = replayed("中a".as_bytes());
        let shown = frame(&term, &ink(), &[], None);
        assert_eq!(
            shown.rows[0].text.chars().take(3).collect::<String>(),
            "中 a"
        );
    }

    #[test]
    fn concealed_cells_paint_only_their_background() {
        let term = replayed("A\x1b[8;4;9me\u{301}中\x1b[41mX\x1b[28mB".as_bytes());
        let shown = frame(&term, &ink(), &[], None);
        let row = &shown.rows[0];
        assert!(row.text.starts_with("A    B"));
        assert!(row.clusters.is_empty());
        let styles: Vec<_> = row
            .runs
            .iter()
            .flat_map(|(len, style)| std::iter::repeat_n(style, *len))
            .collect();
        assert_eq!(styles[1].bg, None);
        assert_eq!(styles[4].bg, Some(gpui::red()));
        assert!(
            styles[1..=4]
                .iter()
                .all(|style| !style.underline && !style.strike)
        );
        assert!(styles[5].underline && styles[5].strike);
    }
}
