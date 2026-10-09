//! Text composition for the terminal. Ely's `Terminal` drops the text an
//! input method is still composing (Vietnamese Telex / VNI, Japanese,
//! Chinese, dead keys), so macOS never sees a composition under way and
//! nothing reaches the shell. `TerminalIme` wraps a terminal, answers the
//! input method in its place, draws the composing text at the cursor and
//! hands the finished text to the terminal.

use std::ops::Range;

use ely_gpui_component::terminal::Terminal;
use ely_gpui_component::theme::{ActiveTheme, TextSize};
use gpui::{
    App, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, Focusable, IntoElement,
    ParentElement, Pixels, Point, Render, Styled, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, canvas, div, fill, font, size,
};

pub struct TerminalIme {
    terminal: Entity<Terminal>,
    /// The text being composed, not yet sent to the shell.
    marked: String,
}

impl TerminalIme {
    pub fn new(terminal: Entity<Terminal>) -> Self {
        Self {
            terminal,
            marked: String::new(),
        }
    }

    fn marked_len_utf16(&self) -> usize {
        self.marked.encode_utf16().count()
    }
}

/// The composing text over the cells at the cursor, underlined as macOS
/// marks it.
fn paint_marked(marked: String, cursor: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let theme = cx.theme();
    let cell = cursor.size;
    let run = TextRun {
        len: marked.len(),
        font: font(theme.mono_family.clone()),
        color: theme.colors.fg,
        background_color: None,
        underline: Some(UnderlineStyle {
            thickness: theme.underline_thickness(),
            color: Some(theme.colors.fg),
            wavy: false,
        }),
        strikethrough: None,
    };
    let text_size = theme.text_size(TextSize::Sm).to_pixels(window.rem_size());
    // The terminal's own ground, so the cells under the text are covered.
    let ground = theme.colors.surface;
    let shaped =
        window
            .text_system()
            .shape_line(marked.into(), text_size, &[run], Some(cell.width));
    let extent = size(shaped.width.max(cell.width), cell.height);
    window.paint_quad(fill(Bounds::new(cursor.origin, extent), ground));
    if let Err(err) = shaped.paint(
        cursor.origin,
        cell.height,
        TextAlign::Left,
        None,
        window,
        cx,
    ) {
        log::error!("terminal: composing text failed to paint: {err:#}");
    }
}

impl Render for TerminalIme {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ime = cx.entity();
        // Painted after the terminal: the last input handler of a frame is
        // the one the window uses.
        let overlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let terminal = ime.read(cx).terminal.clone();
                let focus = terminal.focus_handle(cx);
                window.handle_input(&focus, ElementInputHandler::new(bounds, ime.clone()), cx);
                let marked = ime.read(cx).marked.clone();
                if marked.is_empty() || !focus.is_focused(window) {
                    return;
                }
                let cursor = terminal.update(cx, |terminal, cx| {
                    terminal.bounds_for_range(0..0, bounds, window, cx)
                });
                if let Some(cursor) = cursor {
                    paint_marked(marked, cursor, window, cx);
                }
            },
        )
        .absolute()
        .inset_0();
        div()
            .relative()
            .size_full()
            .child(self.terminal.clone())
            .child(overlay)
    }
}

impl EntityInputHandler for TerminalIme {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked_len_utf16();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked_len_utf16())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.marked.is_empty() {
            self.marked.clear();
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.terminal.update(cx, |terminal, cx| {
            terminal.replace_text_in_range(range, text, window, cx)
        });
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = new_text.to_string();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.terminal.update(cx, |terminal, cx| {
            terminal.bounds_for_range(range, bounds, window, cx)
        })
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
