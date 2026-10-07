//! MonoCode `ContextMeter`: a 14px ring, 2px stroke, muted until the
//! window fills — amber from 75%, red from 90% (`contextUsage.ts`). Hover
//! shows the numbers; where the harness can compact, a click pins them with
//! "Compact now".

use std::f32::consts::{FRAC_PI_2, TAU};

use ely_gpui_component::primitives::Tooltip;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    Pixels, Point, Styled, Window, anchored, canvas, deferred, div, point, prelude::*,
};

use crate::ui::scale::px;

use super::menus::{popover_anchor, popover_surface};
use crate::app::{BenCodeApp, can_compact};
use crate::db::SessionRow;
use crate::ui::sidebar_popovers::popover_glass;
use crate::ui::transcript::turns::format_metric_count;

const SIZE: f32 = 14.0;
const STROKE: f32 = 2.0;
/// Segments in a full circle; the arc is drawn as a polyline.
const SEGMENTS: f32 = 48.0;

/// The ring's colour for `share` of the window used.
pub fn ring_color(share: f32, cx: &App) -> Hsla {
    let colors = &cx.theme().colors;
    if share >= 0.9 {
        colors.danger
    } else if share >= 0.75 {
        colors.warning
    } else {
        colors.fg.opacity(0.45)
    }
}

fn arc(center: Point<Pixels>, radius: Pixels, sweep: f32, color: Hsla, window: &mut Window) {
    let steps = ((SEGMENTS * sweep / TAU).ceil() as usize).max(2);
    let mut path = PathBuilder::stroke(px(STROKE));
    for step in 0..=steps {
        let angle = -FRAC_PI_2 + sweep * step as f32 / steps as f32;
        let at = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if step == 0 {
            path.move_to(at);
        } else {
            path.line_to(at);
        }
    }
    match path.build() {
        Ok(path) => window.paint_path(path, color),
        Err(err) => log::debug!("context ring path: {err:?}"),
    }
}

/// The ring for `share` (0..=1) of the context window.
pub fn context_ring(share: f32, cx: &App) -> impl IntoElement {
    let fill = ring_color(share, cx);
    let track = fill.opacity(0.25);
    let share = share.clamp(0.0, 1.0);
    div().size(px(SIZE)).flex_none().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = bounds.center();
                let radius = bounds.size.width.min(bounds.size.height) / 2.0 - px(STROKE) / 2.0;
                arc(center, radius, TAU, track, window);
                if share > 0.0 {
                    arc(center, radius, TAU * share, fill, window);
                }
            },
        )
        .size_full(),
    )
}

/// The meter's two lines: "N% context used" over "176K / 1M tokens".
fn meter_text(used: i64, window: i64) -> (f32, String, String) {
    let share = (used as f32 / window as f32).clamp(0.0, 1.0);
    let count = |n: i64| format_metric_count(n as f64);
    (
        share,
        format!("{}% context used", (share * 100.0).round()),
        format!("{} / {} tokens", count(used), count(window)),
    )
}

impl BenCodeApp {
    /// The ring, its hover card, and the pinned card with "Compact now".
    pub(super) fn context_meter(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let used = session.context_used?.max(0);
        let window = session.context_window.filter(|w| *w > 0)?;
        let (share, headline, detail) = meter_text(used, window);
        let compactable = can_compact(&session.harness);
        let pinned = compactable && self.composer_menus.context_pinned;
        let card_lines = (headline.clone(), detail.clone());
        let ring = div()
            .id("composer-context")
            .flex_none()
            .when(!pinned, |el| {
                el.tooltip(Tooltip::rich(move |_, cx| {
                    let colors = &cx.theme().colors;
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.0)).child(headline.clone()))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(colors.tooltip_fg.opacity(0.5))
                                .child(detail.clone()),
                        )
                        .into_any_element()
                }))
            })
            .when(compactable, |el| {
                el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    let pin = !this.composer_menus.context_pinned;
                    this.close_composer_popovers(cx);
                    this.composer_menus.context_pinned = pin;
                    cx.notify();
                }))
            })
            .child(context_ring(share, cx));
        Some(
            div()
                .relative()
                .child(popover_anchor(ring, cx))
                .children(pinned.then(|| self.compact_card(session, card_lines, cx)))
                .into_any_element(),
        )
    }

    /// MonoCode's pinned meter: the numbers and "Compact now", locked while
    /// the agent works.
    fn compact_card(
        &self,
        session: &SessionRow,
        (headline, detail): (String, String),
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let busy = self.is_agent_running_in(&session.id);
        let sid = session.id.clone();
        let hover = colors.fg.opacity(0.15);
        let button = div()
            .id("compact-now")
            .mt_1p5()
            .w_full()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(colors.fg.opacity(0.10))
            .text_size(px(11.0))
            .text_color(colors.fg)
            .map(|el| {
                if busy {
                    el.opacity(0.4)
                        .tooltip(Tooltip::text("Wait for the current operation to finish"))
                } else {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.composer_menus.context_pinned = false;
                            this.compact_context(&sid, cx);
                        }))
                }
            })
            .child("Compact now");
        let card = div()
            .id("composer-context-card")
            .w(px(200.0))
            .p_2()
            .rounded(px(12.0))
            .border_1()
            .border_color(colors.border)
            .bg(popover_glass(cx))
            .shadow_xl()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(colors.fg)
                    .child(headline),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(colors.fg.opacity(0.5))
                    .child(detail),
            )
            .child(button);
        deferred(
            anchored()
                .anchor(gpui::Anchor::BottomRight)
                .offset(point(px(14.0), px(-6.0)))
                .snap_to_window()
                .child(popover_surface(card, cx)),
        )
        .with_priority(3)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_reads_share_and_compact_counts() {
        let (share, headline, detail) = meter_text(176_000, 1_000_000);
        assert!((share - 0.176).abs() < 1e-6);
        assert_eq!(headline, "18% context used");
        assert_eq!(detail, "176K / 1M tokens");
    }
}
