//! What the host keeps of a session's output: the screen and scrollback a
//! terminal would show, kept by the emulator Ely's terminal is built on. A
//! client attaching gets them drawn again at its own size (`redraw`), not
//! the bytes the shell wrote: those only mean what they meant at the size
//! they were written for (a prompt's padding, a redraw's cursor moves), and
//! a tab starts at 80 by 24 before its first layout.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

/// The lines of scrollback kept, as many as Ely's terminal keeps.
const SCROLLBACK: usize = 10_000;

pub(super) struct Screen {
    term: Term<Titles>,
    parser: Processor,
    title: Arc<Mutex<Option<String>>>,
}

/// The emulator's events: only the window title is kept. Replies to the
/// program's queries are the attached terminal's to give, not the host's.
struct Titles(Arc<Mutex<Option<String>>>);

impl EventListener for Titles {
    fn send_event(&self, event: Event) {
        let mut title = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match event {
            Event::Title(text) => *title = Some(text),
            Event::ResetTitle => *title = None,
            _ => {}
        }
    }
}

impl Screen {
    pub(super) fn new(cols: u16, rows: u16) -> Self {
        let title = Arc::new(Mutex::new(None));
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size(cols, rows), Titles(title.clone())),
            parser: Processor::new(),
            title,
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        // A synchronized update that never ends is shown as it stands, as
        // a terminal shows it once its wait runs out.
        if self
            .parser
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|until| Instant::now() >= until)
        {
            self.parser.stop_sync(&mut self.term);
        }
    }

    /// A full-screen program is running (vim, less, htop).
    pub(super) fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub(super) fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(size(cols, rows));
    }

    /// The bytes that draw this screen, scrollback first, on an empty
    /// terminal of the same size, and leave it in the same modes. A line
    /// that wrapped is written to the edge and left to wrap, so the
    /// terminal keeps it one line when it is resized.
    pub(super) fn redraw(&mut self) -> Vec<u8> {
        if self.parser.sync_bytes_count() > 0 {
            self.parser.stop_sync(&mut self.term);
        }
        let mode = *self.term.mode();
        let grid = self.term.grid();
        let cols = grid.columns();
        let mut out = Vec::new();
        if mode.contains(TermMode::ALT_SCREEN) {
            out.extend_from_slice(b"\x1b[?1049h");
        }
        let mut pen = Pen::default();
        let top = -(grid.history_size() as i32);
        let bottom = grid.screen_lines() as i32 - 1;
        for line in top..=bottom {
            let row = &grid[Line(line)];
            let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
            let end = if wrapped {
                cols
            } else {
                (0..cols)
                    .rev()
                    .find(|&c| !blank(&row[Column(c)]))
                    .map_or(0, |c| c + 1)
            };
            for col in 0..end {
                write_cell(&mut out, &mut pen, &row[Column(col)]);
            }
            if line < bottom && !wrapped {
                // Plain colours first: a line feed that scrolls fills the
                // new line with the current background.
                pen.reset(&mut out);
                out.extend_from_slice(b"\r\n");
            }
        }
        pen.reset(&mut out);

        let cursor = &grid.cursor;
        let point = cursor.point;
        let at = |out: &mut Vec<u8>, col: usize| {
            out.extend_from_slice(format!("\x1b[{};{}H", point.line.0 + 1, col + 1).as_bytes());
        };
        if cursor.input_needs_wrap {
            // At the edge, waiting to wrap: writing the last cell again
            // leaves the cursor in that state.
            at(&mut out, point.column.0);
            write_cell(&mut out, &mut pen, &grid[point]);
        } else {
            at(&mut out, point.column.0);
        }
        Pen::of(&cursor.template).apply(&mut out, &mut pen);

        for (flag, set) in [
            (TermMode::APP_CURSOR, "?1h"),
            (TermMode::BRACKETED_PASTE, "?2004h"),
            (TermMode::MOUSE_REPORT_CLICK, "?1000h"),
            (TermMode::MOUSE_DRAG, "?1002h"),
            (TermMode::MOUSE_MOTION, "?1003h"),
            (TermMode::SGR_MOUSE, "?1006h"),
            (TermMode::FOCUS_IN_OUT, "?1004h"),
            (TermMode::INSERT, "4h"),
            (TermMode::LINE_FEED_NEW_LINE, "20h"),
        ] {
            if mode.contains(flag) {
                out.extend_from_slice(format!("\x1b[{set}").as_bytes());
            }
        }
        for (flag, reset) in [
            (TermMode::SHOW_CURSOR, "?25l"),
            (TermMode::LINE_WRAP, "?7l"),
            (TermMode::ALTERNATE_SCROLL, "?1007l"),
        ] {
            if !mode.contains(flag) {
                out.extend_from_slice(format!("\x1b[{reset}").as_bytes());
            }
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            out.extend_from_slice(b"\x1b=");
        }
        let title = self.title.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(title) = title.as_deref() {
            out.extend_from_slice(format!("\x1b]2;{title}\x07").as_bytes());
        }
        out
    }
}

fn size(cols: u16, rows: u16) -> TermSize {
    TermSize::new(usize::from(cols.max(2)), usize::from(rows.max(1)))
}

/// A cell an erase leaves: nothing to write at the end of a line.
fn blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.bg == Color::Named(NamedColor::Background)
        && !cell
            .flags
            .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
}

fn write_cell(out: &mut Vec<u8>, pen: &mut Pen, cell: &Cell) {
    // The half a wide character takes after it, or the gap left at a
    // line's end when one did not fit: the terminal makes them itself.
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        return;
    }
    Pen::of(cell).apply(out, pen);
    let mut buf = [0u8; 4];
    out.extend_from_slice(cell.c.encode_utf8(&mut buf).as_bytes());
    for c in cell.zerowidth().unwrap_or_default() {
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
}

/// The attributes a cell is drawn with.
#[derive(Clone, PartialEq)]
struct Pen {
    fg: Color,
    bg: Color,
    underline: Option<Color>,
    flags: Flags,
}

impl Default for Pen {
    fn default() -> Self {
        Self {
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            underline: None,
            flags: Flags::empty(),
        }
    }
}

impl Pen {
    fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            underline: cell.underline_color(),
            flags: cell.flags
                & (Flags::BOLD
                    | Flags::DIM
                    | Flags::ITALIC
                    | Flags::ALL_UNDERLINES
                    | Flags::INVERSE
                    | Flags::HIDDEN
                    | Flags::STRIKEOUT),
        }
    }

    fn reset(&mut self, out: &mut Vec<u8>) {
        Self::default().apply(out, self);
    }

    /// Writes this pen when `current` is another, as one SGR from a reset.
    fn apply(&self, out: &mut Vec<u8>, current: &mut Pen) {
        if self == current {
            return;
        }
        let mut sgr = String::from("\x1b[0");
        for (flag, code) in [
            (Flags::BOLD, ";1"),
            (Flags::DIM, ";2"),
            (Flags::ITALIC, ";3"),
            (Flags::UNDERLINE, ";4"),
            (Flags::DOUBLE_UNDERLINE, ";4:2"),
            (Flags::UNDERCURL, ";4:3"),
            (Flags::DOTTED_UNDERLINE, ";4:4"),
            (Flags::DASHED_UNDERLINE, ";4:5"),
            (Flags::INVERSE, ";7"),
            (Flags::HIDDEN, ";8"),
            (Flags::STRIKEOUT, ";9"),
        ] {
            if self.flags.contains(flag) {
                sgr.push_str(code);
            }
        }
        color(&mut sgr, self.fg, 30, 90, 38);
        color(&mut sgr, self.bg, 40, 100, 48);
        if let Some(underline) = self.underline {
            color(&mut sgr, underline, 0, 0, 58);
        }
        sgr.push('m');
        out.extend_from_slice(sgr.as_bytes());
        *current = self.clone();
    }
}

/// `color` as SGR parameters: `base` + n for the eight colours, `bright` +
/// n for their bright ones, `extended` for an index or RGB. Underline
/// colours (`base` 0) take the extended forms only.
fn color(sgr: &mut String, color: Color, base: u16, bright: u16, extended: u16) {
    let index = match color {
        Color::Spec(rgb) => {
            sgr.push_str(&format!(";{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b));
            return;
        }
        Color::Indexed(index) => index,
        Color::Named(named) => match named as usize {
            n @ 0..=15 => n as u8,
            _ => match named {
                NamedColor::DimBlack
                | NamedColor::DimRed
                | NamedColor::DimGreen
                | NamedColor::DimYellow
                | NamedColor::DimBlue
                | NamedColor::DimMagenta
                | NamedColor::DimCyan
                | NamedColor::DimWhite => (named as usize - NamedColor::DimBlack as usize) as u8,
                // The terminal's own foreground and background.
                _ => return,
            },
        },
    };
    match index {
        0..=7 if base != 0 => sgr.push_str(&format!(";{}", base + u16::from(index))),
        8..=15 if base != 0 => sgr.push_str(&format!(";{}", bright + u16::from(index) - 8)),
        _ => sgr.push_str(&format!(";{extended};5;{index}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen `bytes` leave at `cols` by `rows`, one string per line.
    fn shown(bytes: &[u8], cols: u16, rows: u16) -> Vec<String> {
        let mut screen = Screen::new(cols, rows);
        screen.feed(bytes);
        let grid = screen.term.grid();
        (0..rows as i32)
            .map(|line| {
                let row = &grid[Line(line)];
                (0..cols as usize)
                    .map(|c| row[Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Drawn again on an empty terminal, a screen is what it was.
    fn round_trip(bytes: &[u8], cols: u16, rows: u16) {
        let mut screen = Screen::new(cols, rows);
        screen.feed(bytes);
        let redrawn = screen.redraw();
        assert_eq!(shown(&redrawn, cols, rows), shown(bytes, cols, rows));
        let mut again = Screen::new(cols, rows);
        again.feed(&redrawn);
        assert_eq!(
            again.term.grid().cursor.point,
            screen.term.grid().cursor.point
        );
    }

    #[test]
    fn a_prompt_redrawn_by_the_shell_is_drawn_once() {
        // zsh's PROMPT_SP at 40 columns, a command's output, then the
        // prompt drawn again in place, as zsh does on a size change.
        let mut bytes = b"\x1b[7m%\x1b[27m".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', 39));
        bytes.extend(b"\r \rbenit@mac ~ % ls\r\nfile\r\nbenit@mac ~ % ");
        bytes.extend(b"\r\x1b[Kbenit@mac ~ % ");
        round_trip(&bytes, 40, 6);
        let mut screen = Screen::new(40, 6);
        screen.feed(&bytes);
        let lines = shown(&screen.redraw(), 40, 6);
        assert_eq!(&lines[..3], ["benit@mac ~ % ls", "file", "benit@mac ~ %"]);
    }

    #[test]
    fn a_wrapped_line_stays_one_line_at_another_width() {
        let long = "x".repeat(30);
        let mut screen = Screen::new(20, 4);
        screen.feed(format!("{long}\r\n$ ").as_bytes());
        let redrawn = screen.redraw();
        let mut wide = Screen::new(20, 4);
        wide.feed(&redrawn);
        wide.resize(40, 4);
        let grid = wide.term.grid();
        let first: String = (0..40).map(|c| grid[Line(0)][Column(c)].c).collect();
        assert_eq!(first.trim_end(), long);
    }

    #[test]
    fn scrollback_colours_and_modes_come_back() {
        let mut bytes = Vec::new();
        for n in 0..10 {
            bytes.extend(format!("\x1b[31mline {n}\x1b[0m\r\n").as_bytes());
        }
        bytes.extend(b"\x1b[?2004h\x1b]2;title\x07$ ");
        let mut screen = Screen::new(20, 4);
        screen.feed(&bytes);
        let redrawn = screen.redraw();
        let text = String::from_utf8_lossy(&redrawn);
        assert!(text.contains("\x1b[0;31mline 0"), "{text:?}");
        assert!(text.contains("\x1b[?2004h"));
        assert!(text.contains("\x1b]2;title\x07"));
        round_trip(&bytes, 20, 4);
    }

    #[test]
    fn a_full_screen_program_comes_back_on_the_alternate_screen() {
        let bytes = b"$ vim\r\n\x1b[?1049h\x1b[H\x1b[2Jhello\x1b[3;5H";
        let mut screen = Screen::new(20, 4);
        screen.feed(bytes);
        let redrawn = screen.redraw();
        let mut again = Screen::new(20, 4);
        again.feed(&redrawn);
        assert!(again.term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(shown(&redrawn, 20, 4)[0], "hello");
        assert_eq!(
            again.term.grid().cursor.point,
            screen.term.grid().cursor.point
        );
    }
}
