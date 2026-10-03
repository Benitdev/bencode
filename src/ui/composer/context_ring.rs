//! MonoCode `ContextMeter` ring: 14px, 2px stroke, muted until the window
//! fills — amber from 75%, red from 90% (`contextUsage.ts`).

use std::f32::consts::{FRAC_PI_2, TAU};

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    App, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, Window, canvas, div, point,
    prelude::*, px,
};

const SIZE: Pixels = px(14.0);
const STROKE: Pixels = px(2.0);
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
    let mut path = PathBuilder::stroke(STROKE);
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
    div().size(SIZE).flex_none().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = bounds.center();
                let radius = bounds.size.width.min(bounds.size.height) / 2.0 - STROKE / 2.0;
                arc(center, radius, TAU, track, window);
                if share > 0.0 {
                    arc(center, radius, TAU * share, fill, window);
                }
            },
        )
        .size_full(),
    )
}
