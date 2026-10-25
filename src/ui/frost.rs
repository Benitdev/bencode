//! MonoCode `.popover-backdrop` / `GlassBackdrop`: a `backdrop-blur-xl`
//! glass tinted `content` at 2% (the base background alone in the light
//! theme), after MonoCode's GPUI port (`popover.rs` `paint_glass`, and
//! `modal.rs` `surface` for dialogs).
//!
//! GPUI cannot blur what lies behind an element; the vendored Metal renderer
//! can (`vendor/gpui-pre-apple`): a quad whose border colour is
//! `backdrop_blur_marker(sigma)` first has what is already drawn beneath it
//! blurred, then its own fill laid on as the tint. The light theme and other
//! platforms paint an opaque fill instead.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{App, Hsla, IntoElement, Styled, canvas};

use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_glass;

/// MonoCode `backdrop-blur-xl`: 24px.
#[cfg(target_os = "macos")]
const BLUR: f32 = 24.0;

/// The inner radius of a `rounded-xl border` frame (12px less its border).
pub(crate) const POPOVER_RADIUS: f32 = 11.0;

/// The frosted surface of a popover: the first child of its frame, filling
/// the frame inside its border with corners of `radius`.
pub(crate) fn frost(radius: f32, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let tint = theme.is_dark().then(|| theme.colors.fg.opacity(0.02));
    surface(radius, tint, popover_glass(cx))
}

/// MonoCode `modal.rs` `surface`: a dialog card's frost, the base
/// background at 55% over the blur, so the window behind shows through
/// instead of the card standing dark on the scrim. `opaque` is the card's
/// own fill where it cannot blur.
pub(crate) fn dialog_frost(radius: f32, opaque: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let tint = theme.is_dark().then(|| theme.colors.bg.opacity(0.55));
    surface(radius, tint, opaque)
}

/// `tint` over a backdrop blur where there is one, else `opaque`.
fn surface(radius: f32, tint: Option<Hsla>, opaque: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let quad = match tint {
                #[cfg(target_os = "macos")]
                Some(tint) => gpui::fill(bounds, tint).border_color(
                    gpui_apple::metal_renderer::backdrop_blur_marker(
                        f32::from(px(BLUR)) * window.scale_factor(),
                    ),
                ),
                _ => gpui::fill(bounds, opaque),
            };
            window.paint_quad(quad.corner_radii(px(radius)));
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
