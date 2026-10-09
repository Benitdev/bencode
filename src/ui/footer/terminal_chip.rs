//! MonoCode `UsageFooter` `RunningTerminalChip`: in place of the Terminal
//! button while jobs run in the project's terminals, their names after a
//! live mark. A click shows or hides the terminal; with several running and
//! the dock hidden it lists them to pick one.

use std::time::{Duration, Instant};

use ely_gpui_component::primitives::Tooltip;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, Hsla, IntoElement, ParentElement, Styled, div, prelude::*, rgb};

use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::terminal_pane::{POLL_EVERY, RunningTerminal, chip_label};

/// MonoCode `.terminal-live-bar`'s `#e39b4a`.
const LIVE_BAR: u32 = 0xe39b4a;

/// One start for every live mark, so they step together.
fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// MonoCode `terminal-live-bar-N`: over 3.2s, bar N lights at N quarters
/// and all go dark together; dark throughout with reduced motion.
fn bar_lit(bar: usize, elapsed: Duration, quarter: Duration, reduced: bool) -> bool {
    if reduced {
        return false;
    }
    let step = (elapsed.as_millis() / quarter.as_millis()) % 4;
    step as usize > bar
}

/// MonoCode `TerminalLiveMark`: three 4×8 bars, 2px apart.
fn live_mark(reduced: bool) -> impl IntoElement {
    let color: Hsla = rgb(LIVE_BAR).into();
    let elapsed = epoch().elapsed();
    div()
        .flex()
        .flex_none()
        .items_end()
        .gap(px(2.0))
        .h(px(10.0))
        .children((0..3).map(|bar| {
            let lit = bar_lit(bar, elapsed, POLL_EVERY, reduced);
            div()
                .w(px(4.0))
                .h(px(8.0))
                .bg(color.opacity(if lit { 0.85 } else { 0.4 }))
        }))
}

/// MonoCode's `aria-label`, shown here as the tooltip's first line.
fn chip_title(terminals: &[RunningTerminal], open: bool) -> String {
    let action = match (terminals, open) {
        ([one], true) => format!("Hide {}", one.process),
        ([one], false) => format!("Show {}", one.process),
        (_, true) => "Hide running terminals".to_string(),
        (many, false) => format!("{} terminals are running processes", many.len()),
    };
    let lines = terminals
        .iter()
        .map(|t| format!("\"{}\" in {}", t.process, t.label))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{action}\n{lines}")
}

impl BenCodeApp {
    pub(super) fn render_running_terminal_chip(
        &self,
        terminals: Vec<RunningTerminal>,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let fg = cx.theme().colors.fg;
        let open = self.is_terminal_open();
        let first = terminals.first().map(|t| t.id);
        let many = terminals.len() > 1;
        div()
            .relative()
            .flex_none()
            .child(
                div()
                    .id("footer-running-terminals")
                    .flex()
                    .min_w_0()
                    .max_w(px(256.0))
                    .items_center()
                    .gap(px(6.0))
                    .h(px(20.0))
                    .px(px(4.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .when(self.running_terminals_menu_open(), |el| {
                        el.bg(fg.opacity(0.10))
                    })
                    .hover(move |s| s.bg(fg.opacity(0.10)).text_color(fg))
                    // The open list closes on this press, as on any press
                    // outside it; the click must then not reopen it.
                    .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                        this.terminals.running_menu_at_press = this.running_terminals_menu_open();
                    }))
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                        window.prevent_default()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if std::mem::take(&mut this.terminals.running_menu_at_press) {
                            this.close_terminal_menu(cx);
                        } else if open || !many {
                            this.close_terminal_menu(cx);
                            if let Some(id) = first {
                                this.toggle_running_terminal(id, cx);
                            }
                        } else {
                            this.toggle_running_terminals_menu(cx);
                        }
                    }))
                    .tooltip(Tooltip::text(chip_title(&terminals, open)))
                    .child(live_mark(cx.reduce_motion()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().mono_family.clone())
                            .text_size(px(10.0))
                            .child(chip_label(&terminals)),
                    ),
            )
            .children(self.render_running_terminals_menu(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_light_one_quarter_at_a_time() {
        let q = Duration::from_millis(800);
        let lit = |ms: u64| {
            (0..3)
                .map(|bar| bar_lit(bar, Duration::from_millis(ms), q, false))
                .collect::<Vec<_>>()
        };
        assert_eq!(lit(0), [false, false, false]);
        assert_eq!(lit(800), [true, false, false]);
        assert_eq!(lit(1600), [true, true, false]);
        assert_eq!(lit(2400), [true, true, true]);
        assert_eq!(lit(3200), [false, false, false]);
        assert!(!bar_lit(0, Duration::from_millis(900), q, true));
    }

    #[test]
    fn title_says_what_a_click_does() {
        let vite = RunningTerminal {
            id: 1,
            process: "vite".into(),
            label: "web".into(),
        };
        let jest = RunningTerminal {
            id: 2,
            process: "jest".into(),
            label: "api".into(),
        };
        assert_eq!(
            chip_title(&[vite.clone()], false),
            "Show vite\n\"vite\" in web"
        );
        assert_eq!(
            chip_title(&[vite.clone()], true),
            "Hide vite\n\"vite\" in web"
        );
        assert_eq!(
            chip_title(&[vite, jest], false),
            "2 terminals are running processes\n\"vite\" in web\n\"jest\" in api"
        );
    }
}
