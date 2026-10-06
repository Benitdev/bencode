//! Direct manipulation on the rail: dragging a project row to a new slot
//! in its list (MonoCode `useAnimatedReorder`, the rows between sliding
//! over) and dragging the right edge to resize (MonoCode `useDragResize`).

use std::time::{Duration, Instant};

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    Animation, AnimationExt, AnyElement, Context, DragMoveEvent, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*, px, relative,
};

use super::model::{self, move_item, reorder_subset, resized_rail_width};
use super::state::RailReorder;
use crate::app::BenCodeApp;
use crate::ui::mascot::{Mascot, pixel_sprite};
use crate::ui::motion::cubic_bezier;

/// MonoCode `--motion-reorder-duration`.
pub(super) const REORDER_MOTION: Duration = Duration::from_millis(160);
/// A row's pitch: `h-8` plus the list's `gap-px`.
pub(super) const ROW_PITCH: f32 = 33.0;

/// Drag payload of a project row; it draws the row under the pointer.
#[derive(Clone)]
pub struct DraggedRailProject {
    pub list: String,
    pub path: String,
    pub label: String,
    pub color: Hsla,
    pub mascot: &'static Mascot,
    pub width: f32,
}

impl Render for DraggedRailProject {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        // The held row: `bg-selection-strong` on the rail's glass.
        let strong = fg.opacity(if theme.is_dark() { 0.12 } else { 0.07 });
        div()
            .flex()
            .items_center()
            .gap_2()
            .w(px(self.width))
            .h(px(32.0))
            .px_2()
            .rounded(px(6.0))
            .bg(theme.colors.bg.blend(strong))
            .shadow_md()
            .text_color(fg)
            .text_size(px(14.0))
            .font_weight(FontWeight::MEDIUM)
            .line_height(relative(1.25))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .size_4()
                    .items_center()
                    .justify_center()
                    .child(pixel_sprite(&self.mascot.rest, px(12.0), self.color, false)),
            )
            .child(div().min_w_0().flex_1().truncate().child(self.label.clone()))
    }
}

/// Drag payload of the rail's resize sash.
#[derive(Clone)]
pub struct RailResize;

impl Render for RailResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// `list` as shown while `reorder` holds one of its rows over a slot.
pub(super) fn preview_order(list: &str, ids: &[String], reorder: Option<&RailReorder>) -> Vec<String> {
    match reorder.filter(|r| r.list == list) {
        Some(r) => match ids.iter().position(|id| *id == r.path) {
            Some(from) => move_item(ids, from, r.to),
            None => ids.to_vec(),
        },
        None => ids.to_vec(),
    }
}

impl BenCodeApp {
    /// The rail's width: the drag's while the sash is held.
    pub(super) fn rail_width(&self) -> f32 {
        self.rail_ui
            .drag_width
            .unwrap_or_else(|| self.settings.rail.rail_width())
    }

    /// Follows the sash (registered on the rail; drag moves reach it
    /// wherever the pointer is).
    pub(super) fn track_rail_resize(&mut self, event: &DragMoveEvent<RailResize>, window: &Window, cx: &mut Context<Self>) {
        let (start_x, start_width) = self.rail_ui.drag_origin;
        let width = resized_rail_width(
            start_width,
            f32::from(event.event.position.x) - start_x,
            f32::from(window.viewport_size().width),
        );
        if self.rail_ui.drag_width != Some(width) {
            self.rail_ui.drag_width = Some(width);
            cx.notify();
        }
    }

    /// Pointer up anywhere: the resize is saved and a held row lands
    /// where it is previewed (MonoCode commits on `pointerup`).
    pub(super) fn finish_rail_drags(&mut self, cx: &mut Context<Self>) {
        if let Some(width) = self.rail_ui.drag_width.take() {
            self.update_rail_prefs(
                |prefs| model::RailPrefs {
                    project_rail_width: Some(width),
                    ..prefs.clone()
                },
                cx,
            );
            cx.notify();
        }
        if let Some(reorder) = self.rail_ui.reorder.take() {
            self.commit_rail_reorder(&reorder, cx);
            cx.notify();
        }
    }

    /// MonoCode's resize separator: `absolute inset-y-0 -right-px w-1.5
    /// cursor-col-resize`, `bg-content/15` held, `hover:bg-content/10`; a
    /// double click restores the default width.
    pub(super) fn render_rail_sash(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let dragging = self.rail_ui.drag_width.is_some();
        div()
            .id("rail-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-1.0))
            .w(px(6.0))
            .cursor_col_resize()
            .when(dragging, |el| el.bg(fg.opacity(0.15)))
            .when(!dragging, |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.update_rail_prefs(
                            |prefs| model::RailPrefs {
                                project_rail_width: None,
                                ..prefs.clone()
                            },
                            cx,
                        );
                        return;
                    }
                    this.rail_ui.drag_origin = (f32::from(event.position.x), this.rail_width());
                }),
            )
            .on_drag(RailResize, |drag, _, _, cx| cx.new(|_| drag.clone()))
    }

    /// Follows a row held over `list` (`ids` in shown order, the list's
    /// first row at `top`), sliding the rows it passes.
    pub(super) fn track_rail_reorder(
        &mut self,
        list: &str,
        ids: &[String],
        event: &DragMoveEvent<DraggedRailProject>,
        cx: &mut Context<Self>,
    ) {
        let dragged = event.drag(cx).clone();
        if dragged.list != list || ids.len() < 2 {
            return;
        }
        let y = f32::from(event.event.position.y - event.bounds.origin.y);
        let to = ((y / ROW_PITCH).floor().max(0.0) as usize).min(ids.len() - 1);
        let next = RailReorder {
            list: list.to_string(),
            path: dragged.path,
            to,
        };
        if self.rail_ui.reorder.as_ref() == Some(&next) {
            return;
        }
        let before = preview_order(list, ids, self.rail_ui.reorder.as_ref());
        let after = preview_order(list, ids, Some(&next));
        let now = Instant::now();
        for (new_ix, id) in after.iter().enumerate() {
            let Some(old_ix) = before.iter().position(|b| b == id) else {
                continue;
            };
            if old_ix != new_ix && *id != next.path {
                self.rail_ui.slide_seq += 1;
                let offset = (old_ix as f32 - new_ix as f32) * ROW_PITCH;
                self.rail_ui.slides.insert(id.clone(), (offset, self.rail_ui.slide_seq, now));
            }
        }
        self.rail_ui.slides.retain(|_, (_, _, at)| at.elapsed() < REORDER_MOTION);
        self.rail_ui.reorder = Some(next);
        cx.notify();
    }

    /// MonoCode `onReorderPinned` / `onReorderProjects`: the list's new
    /// order takes its members' slots in the saved rail order.
    fn commit_rail_reorder(&mut self, reorder: &RailReorder, cx: &mut Context<Self>) {
        let order = self.rail_order();
        let sections = self.rail_sections_for(&order);
        let ids = match reorder.list.as_str() {
            "pinned" => sections.pinned,
            "projects" => sections.ungrouped,
            other => other
                .strip_prefix("group:")
                .and_then(|id| sections.groups.into_iter().find(|(g, _)| g.id == id))
                .map(|(_, items)| items)
                .unwrap_or_default(),
        };
        let Some(from) = ids.iter().position(|id| *id == reorder.path) else {
            return;
        };
        if from == reorder.to {
            return;
        }
        let next = reorder_subset(&order, &move_item(&ids, from, reorder.to));
        self.update_rail_prefs(
            |prefs| model::RailPrefs {
                project_rail_order: next.clone(),
                ..prefs.clone()
            },
            cx,
        );
    }

    /// A row in its slot; one passed by the held row slides from where it
    /// was (MonoCode `transform ${duration}ms ${--motion-ease-out}`).
    pub(super) fn slide_rail_row(&self, path: &str, row: AnyElement) -> AnyElement {
        let slide = self
            .rail_ui
            .slides
            .get(path)
            .filter(|(_, _, at)| at.elapsed() < REORDER_MOTION)
            .copied();
        let Some((offset, seq, _)) = slide else {
            return row;
        };
        div()
            .relative()
            .child(row)
            .with_animation(
                SharedString::from(format!("rail-slide-{path}-{seq}")),
                Animation::new(REORDER_MOTION).with_easing(cubic_bezier(0.22, 1.0, 0.36, 1.0)),
                move |el, delta| el.top(px(offset * (1.0 - delta))),
            )
            .into_any_element()
    }
}
