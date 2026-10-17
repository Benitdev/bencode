//! MonoCode `ProjectBackgroundDialog` with `locked`, as a Mono's rail menu
//! opens it: only the image is the user's choice, shown behind the Mono's
//! chat under the Haze at the default dimming (`app/mono_background.rs`).

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    StatefulInteractiveElement, Styled, StyledImage, div, img, prelude::*, rgb,
};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::appearance::CHAT_BACKGROUND_OPACITY_DEFAULT;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    /// Full-window: in `BenCodeApp::render`'s `dialogs` list.
    pub(crate) fn render_mono_background_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.mono_backgrounds.dialog.clone()?;
        let mono = self.mono(&id)?;
        let name = mono.display_name();
        let has_image = mono.background.is_some();
        let busy = self.mono_backgrounds.busy;
        let fg = cx.theme().colors.fg;
        let red_400: gpui::Hsla = rgb(0xf87171).into();

        // `overflow-hidden rounded-xl border border-content/10
        // bg-background-base`, `h-40`.
        let preview = div()
            .relative()
            .h(px(160.0))
            .w_full()
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(fg.fill(Fill::Strong))
            .bg(cx.theme().colors.bg)
            .map(|el| match self.mono_background_image(&id) {
                Some(image) => el.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(CHAT_BACKGROUND_OPACITY_DEFAULT),
                ),
                None if has_image => el,
                None => el
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child("No background selected"),
            });

        let choose_id = id.clone();
        let choose = app_callback(cx, move |this, cx| this.choose_mono_background(&choose_id, cx));
        // `mt-2 w-full justify-center gap-1.5 rounded-md border
        // border-content/10 px-2.5 py-1.5 text-[12px] text-content/70`
        let choose_button = div()
            .id("mono-background-choose")
            .mt_2()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(fg.fill(Fill::Strong))
            .px(px(10.0))
            .py(px(6.0))
            .text_size(Text::Base.px())
            .text_color(fg.ink(Ink::Body))
            .when(busy, |el| el.opacity(0.4))
            .when(!busy, |el| {
                el.cursor_pointer()
                    .hover(|el| el.bg(fg.fill(Fill::Strong)).text_color(fg))
                    .on_click(move |_, window, cx| choose(window, cx))
            })
            .when(busy, |el| {
                el.child(spinning_icon(
                    "mono-background-busy".into(),
                    IconName::LoaderCircle,
                    IconSize::Sm,
                    fg.ink(Ink::Body),
                ))
            })
            .child(if has_image { "Change image" } else { "Choose image" });

        let mut body = div()
            .p_4()
            .child(preview)
            .child(choose_button)
            .child(
                div()
                    .mt(px(6.0))
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(format!("Shown behind {name}'s chat, dimmed with the Haze effect.")),
            )
            .children(self.mono_backgrounds.error.clone().map(|error| {
                div()
                    .mt(px(6.0))
                    .text_size(Text::Base.px())
                    .text_color(red_400)
                    .child(error)
            }));
        if has_image {
            let remove_id = id.clone();
            let remove = app_callback(cx, move |this, cx| this.remove_mono_background(&remove_id, cx));
            // `w-full rounded-md border border-content/10 text-red-400
            // hover:border-red-400/40 hover:bg-red-400/10`
            body = body.child(
                div()
                    .id("mono-background-remove")
                    .mt_5()
                    .w_full()
                    .flex()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(fg.fill(Fill::Strong))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(Text::Base.px())
                    .text_color(red_400)
                    .when(busy, |el| el.opacity(0.4))
                    .when(!busy, |el| {
                        el.cursor_pointer()
                            .hover(move |el| el.border_color(red_400.opacity(0.4)).bg(red_400.opacity(0.1)))
                            .on_click(move |_, window, cx| remove(window, cx))
                    })
                    .child("Remove background image"),
            );
        }

        let close = app_callback(cx, |this, cx| this.close_mono_background(cx));
        Some(
            Dialog::new("mono-background", "Background Image", close)
                .detail(format!("Choose a background image for {name}"))
                .child(body)
                .action(|close| {
                    Button::new("mono-background-done", "Done")
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .into_any_element(),
        )
    }
}
