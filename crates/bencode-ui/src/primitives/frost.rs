//! Frosted glass, after MonoCode's GPUI port (`popover.rs` `paint_glass`,
//! `modal.rs` `surface`): a `backdrop-blur-xl` under a light tint, so what
//! lies behind a popup or a dialog shows through blurred.
//!
//! GPUI cannot blur what lies behind an element; BenCode's vendored Metal
//! renderer can (`vendor/gpui-pre-apple`): a quad whose border colour is
//! `backdrop_blur_marker(sigma, tint_alpha)` first has what is already drawn
//! beneath it blurred, then its own fill laid on as the tint. A card fading
//! in or out by opacity fades its blur with it (the renderer reads the fade
//! off the tint). The light theme and other
//! platforms paint the opaque fill instead.
//!
//! A frost is the first child of its card, filling it inside its border.

use gpui::{App, Hsla, IntoElement, Pixels, Styled, canvas};

use crate::theme::ActiveTheme;

/// MonoCode `backdrop-blur-xl`: 24px at the interface's scale.
#[cfg(target_os = "macos")]
const BLUR: f32 = 24.0;

/// How much of the base background a popup's frost lays over its blur.
/// MonoCode's 2% of the content colour reads only over BenCode's own dark
/// panels: over a bright picture or page the blur turns light and the
/// light text vanishes. At this, a menu over white stays dark enough to
/// read, and over the dark panels it looks as before.
const POPOVER_BASE: f32 = 0.72;

/// A popup's frost: the base background, lifted by the content colour at
/// 2% (MonoCode `.popover-backdrop`), over the blur; `opaque` where there
/// is no blur.
pub fn popover(radius: Pixels, opaque: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    surface(
        radius,
        theme.is_dark().then(|| {
            colors
                .bg
                .blend(colors.fg.opacity(0.02))
                .opacity(POPOVER_BASE)
        }),
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
                    gpui::fill(bounds, tint).border_color(
                        gpui_apple::metal_renderer::backdrop_blur_marker(sigma, tint.a),
                    )
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
