//! The inspector (BenCode's own): Changes or the Explorer at the
//! workspace's right, under the title bar. The navigator keeps the threads
//! in view while it shows; the title bar's toggles pick what it holds
//! (⇧⌘B shows or hides it, ⇧⌘E opens the Explorer, ⌃⇧G Changes) and its
//! left edge drags to resize. Whether it shows, what and how wide are kept
//! in `settings.json`.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::{BenCodeApp, InspectorMode};
use crate::ui::region::Region;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Tone as _};

pub const INSPECTOR_DEFAULT_WIDTH: f32 = 320.0;
const INSPECTOR_MIN_WIDTH: f32 = 260.0;
const INSPECTOR_MAX_WIDTH: f32 = 560.0;

/// Drag payload of the inspector's resize sash.
#[derive(Clone, Debug)]
pub struct InspectorResize {
    pub start_x: f32,
    pub start_width: f32,
}

impl gpui::Render for InspectorResize {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The width kept in `settings.json`, within the inspector's range.
pub fn saved_width(width: Option<f32>) -> f32 {
    width
        .filter(|width| width.is_finite())
        .map_or(INSPECTOR_DEFAULT_WIDTH, |width| {
            width
                .round()
                .clamp(INSPECTOR_MIN_WIDTH, INSPECTOR_MAX_WIDTH)
        })
}

/// The width a drag to `x` gives: the sash is the inspector's left edge, so
/// it widens leftwards. Never more than half the window.
pub fn resized_width(drag: &InspectorResize, x: f32, window_width: f32) -> f32 {
    let max = INSPECTOR_MAX_WIDTH
        .min(window_width * 0.5)
        .max(INSPECTOR_MIN_WIDTH);
    (drag.start_width + drag.start_x - x)
        .round()
        .clamp(INSPECTOR_MIN_WIDTH, max)
}

impl BenCodeApp {
    /// Opens the inspector on `mode` (MonoCode's sidebar tab click, which
    /// refreshes what Changes reads).
    pub(crate) fn show_inspector(&mut self, mode: InspectorMode, cx: &mut Context<Self>) {
        let opening = !self.inspector_open;
        let changed = opening || self.inspector_mode != mode;
        self.inspector_open = true;
        self.inspector_mode = mode;
        if mode == InspectorMode::Changes {
            self.refresh_workspace(cx);
            self.reload_branch_pr_if_stale(cx);
        }
        if changed {
            self.save_settings(cx);
        }
        if opening {
            // The shell lays the column out.
            cx.notify();
        } else {
            self.redraw_regions(
                &[Region::Inspector, Region::Titlebar, Region::CompactRail],
                cx,
            );
        }
    }

    /// The title bar's toggles: `mode` shown, or the inspector hidden when
    /// it already shows `mode`.
    pub(crate) fn toggle_inspector_mode(&mut self, mode: InspectorMode, cx: &mut Context<Self>) {
        if self.inspector_open && self.inspector_mode == mode {
            self.set_inspector_open(false, cx);
        } else {
            self.show_inspector(mode, cx);
        }
    }

    pub(crate) fn set_inspector_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if open {
            self.show_inspector(self.inspector_mode, cx);
        } else if self.inspector_open {
            self.inspector_open = false;
            self.save_settings(cx);
            cx.notify();
        }
    }

    pub fn render_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.inspector_width))
            .h_full()
            .bg(glass.body(colors.bg))
            .border_l_1()
            .border_color(colors.fg.fill(Fill::Line))
            .line_height(gpui::relative(1.5))
            .child(match self.inspector_mode {
                InspectorMode::Files if self.project_search.open => {
                    self.render_project_search(cx).into_any_element()
                }
                InspectorMode::Files => self.render_file_tree(cx).into_any_element(),
                InspectorMode::Changes => self.render_git_changes_panel(cx).into_any_element(),
            })
            .child(self.render_inspector_sash(cx))
    }

    /// The resize separator on the inspector's left edge; a double click
    /// restores the default width.
    fn render_inspector_sash(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let dragging = self.inspector_resizing;
        let width = self.inspector_width;
        div()
            .id("inspector-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-3.0))
            .w(px(6.0))
            .cursor_col_resize()
            .when(dragging, |el| el.bg(fg.fill(Fill::Bold)))
            .when(!dragging, |el| {
                el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
            })
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.inspector_width = INSPECTOR_DEFAULT_WIDTH;
                        this.save_settings(cx);
                        cx.notify();
                    }
                    this.inspector_drag_x = crate::ui::scale::logical(event.position.x);
                }),
            )
            .on_drag(
                InspectorResize {
                    start_x: 0.0,
                    start_width: width,
                },
                |drag, _, _, cx| {
                    let drag = drag.clone();
                    cx.new(|_| drag)
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_width_stays_in_range() {
        assert_eq!(saved_width(None), INSPECTOR_DEFAULT_WIDTH);
        assert_eq!(saved_width(Some(f32::NAN)), INSPECTOR_DEFAULT_WIDTH);
        assert_eq!(saved_width(Some(100.0)), INSPECTOR_MIN_WIDTH);
        assert_eq!(saved_width(Some(400.4)), 400.0);
        assert_eq!(saved_width(Some(2000.0)), INSPECTOR_MAX_WIDTH);
    }

    #[test]
    fn resizing_widens_leftwards_within_half_the_window() {
        let drag = InspectorResize {
            start_x: 1000.0,
            start_width: 320.0,
        };
        assert_eq!(resized_width(&drag, 900.0, 2000.0), 420.0);
        assert_eq!(resized_width(&drag, 1100.0, 2000.0), 260.0);
        assert_eq!(resized_width(&drag, 200.0, 2000.0), 560.0);
        assert_eq!(resized_width(&drag, 200.0, 800.0), 400.0);
    }
}
