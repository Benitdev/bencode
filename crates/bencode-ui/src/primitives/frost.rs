//! Frosted glass, after MonoCode's GPUI port (`popover.rs` `paint_glass`,
//! `modal.rs` `surface`): a `backdrop-blur-xl` under a light tint, so what
//! lies behind a popup or a dialog shows through blurred.
//!
//! GPUI cannot blur what lies behind an element; BenCode's vendored Metal
//! renderer can (`vendor/gpui-pre-apple`): a quad whose border colour is
//! `backdrop_blur_marker(sigma)` first has what is already drawn beneath it
//! blurred, then its own fill laid on as the tint. The light theme and other
//! platforms paint the opaque fill instead.
//!
//! A frost is the first child of its card, filling it inside its border.

use gpui::{App, Hsla, IntoElement, Pixels, Styled, canvas};

use crate::theme::ActiveTheme;

/// MonoCode `backdrop-blur-xl`: 24px at the interface's scale.
#[cfg(target_os = "macos")]
const BLUR: f32 = 24.0;

/// A popup's frost: the content colour at 2% over the blur (MonoCode
/// `.popover-backdrop`), `opaque` where there is no blur.
pub fn popover(radius: Pixels, opaque: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    surface(
        radius,
        theme.is_dark().then(|| theme.colors.fg.opacity(0.02)),
        opaque,
    )
}

/// A dialog card's frost: the base background at 30% over the blur, so the
/// window behind shows through instead of the card standing dark on the
/// scrim; `opaque` where there is no blur.
pub fn dialog(radius: Pixels, opaque: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    surface(
        radius,
        theme.is_dark().then(|| theme.colors.bg.opacity(0.3)),
        opaque,
    )
}

/// `tint` over a backdrop blur where there is one, else `opaque`.
fn surface(radius: Pixels, tint: Option<Hsla>, opaque: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let quad = match tint {
                #[cfg(target_os = "macos")]
                Some(tint) => {
                    // The rem follows the interface scale; 16px is 1rem at 100%.
                    let sigma = BLUR * f32::from(window.rem_size()) / 16.0 * window.scale_factor();
                    gpui::fill(bounds, tint)
                        .border_color(gpui_apple::metal_renderer::backdrop_blur_marker(sigma))
                }
                _ => gpui::fill(bounds, opaque),
            };
            window.paint_quad(quad.corner_radii(radius));
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
