//! MonoCode `TerminalSpinner`: a braille spinner, one frame every 80ms.

use std::time::Duration;

use gpui::{Animation, AnimationExt, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px};

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The spinner in `color`; `id` must be unique among animations on screen.
pub fn terminal_spinner(id: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    div()
        .w(px(12.0))
        .flex_none()
        .text_size(px(11.0))
        .line_height(px(11.0))
        .text_color(color)
        .with_animation(
            id.into(),
            Animation::new(Duration::from_millis(800)).repeat(),
            |el, delta| {
                let frame = (delta * FRAMES.len() as f32) as usize;
                el.child(FRAMES[frame.min(FRAMES.len() - 1)])
            },
        )
}
