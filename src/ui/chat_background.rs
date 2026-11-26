//! MonoCode GPUI `ProjectBackground`: cover-cropped artwork under the chat
//! and its settings preview. The Metal sprite fades after cropping.

use std::sync::Arc;

use gpui::{Img, ObjectFit, Pixels, RenderImage, Styled, StyledImage, img};

pub(crate) fn artwork(
    image: Arc<RenderImage>,
    opacity: f32,
    radius: Pixels,
    _empty: bool,
    _haze: bool,
) -> Img {
    let artwork = img(image)
        .absolute()
        .inset_0()
        .size_full()
        .rounded(radius)
        .object_fit(ObjectFit::Cover)
        .opacity(opacity);

    #[cfg(target_os = "macos")]
    let artwork = {
        let corners = gpui_apple::metal_renderer::chat_background_corners(radius, _empty, _haze);
        artwork
            .rounded_tl(corners.top_left)
            .rounded_tr(corners.top_right)
            .rounded_bl(corners.bottom_left)
    };

    artwork
}
