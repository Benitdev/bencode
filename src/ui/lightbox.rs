//! Full-window image preview (MonoCode `ImageLightbox`): the image fitted
//! on a dark backdrop, a round close button at the top right; Esc, the
//! button or a click on the backdrop closes it.

use std::path::PathBuf;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::IconSize;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement,
    Styled, StyledImage, deferred, div, prelude::*, rgba,
};

use crate::app::BenCodeApp;
use crate::ui::scale::px;

/// The largest the preview may be drawn, in logical pixels: a window's
/// width, so a screenshot keeps its full resolution.
const LIGHTBOX: f32 = 2048.0;

impl BenCodeApp {
    pub fn open_lightbox(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(shown) = self.lightbox.replace(path)
            && self.lightbox.as_ref() != Some(&shown)
        {
            crate::ui::thumbnail::release(shown, LIGHTBOX, cx);
        }
        cx.notify();
    }

    /// True when a preview was open. The full-size image is let go.
    pub fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.lightbox.take() else {
            return false;
        };
        crate::ui::thumbnail::release(path, LIGHTBOX, cx);
        cx.notify();
        true
    }

    pub fn render_lightbox(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let path = self.lightbox.clone()?;
        let close = div()
            .id("lightbox-close")
            .absolute()
            .top_4()
            .right_4()
            .size(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(rgba(0xffffff26))
            .bg(rgba(0x00000073))
            .shadow_lg()
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0x000000a6)))
            .tooltip(Tooltip::text("Close"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_lightbox(cx);
            }))
            .child(
                Icon::new(IconName::X)
                    .size(IconSize::Sm)
                    .color(rgba(0xffffffcc)),
            );
        Some(
            deferred(
                div()
                    .id("lightbox")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .p_6()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x000000d9))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_lightbox(cx);
                        }),
                    )
                    .child(
                        // Only the backdrop closes; the image itself does not.
                        div()
                            .max_w_full()
                            .max_h_full()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                crate::ui::thumbnail::thumbnail(path, LIGHTBOX)
                                    .max_w_full()
                                    .max_h_full()
                                    .object_fit(ObjectFit::Contain)
                                    .shadow_2xl(),
                            ),
                    )
                    .child(close),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }
}
