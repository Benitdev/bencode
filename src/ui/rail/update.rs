//! MonoCode `SidebarUpdateFooter` (`SidebarUpdate.tsx`, `UpdateRailCard.tsx`):
//! under the Working agents card, the "Updated to" card a restart leaves
//! and the "Update to" button while there is an update to act on. Nothing
//! shows otherwise; manual checks live in Settings and the menu. The state
//! is `app/updater.rs`.

use std::sync::{Arc, LazyLock};

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Image, ImageFormat, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, img, prelude::*,
    relative,
};

use crate::app::BenCodeApp;
use crate::app::updater::Phase;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;

/// MonoCode's `/monocode.png` beside "Updated to": BenCode's own icon.
static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../../assets/app-icon.png").to_vec(),
    ))
});

impl BenCodeApp {
    /// The footer, or nothing when neither part has anything to show.
    /// `bottom_spacing` is for where nothing follows it (the sidebar's
    /// foot while the rail is closed).
    pub(crate) fn render_update_footer(
        &self,
        bottom_spacing: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let card = self
            .updater
            .installed
            .clone()
            .map(|version| self.render_updated_card(version, cx));
        let button = self
            .updater
            .actionable()
            .then(|| self.render_update_button(cx));
        if card.is_none() && button.is_none() {
            return None;
        }
        // `flex flex-col gap-1.5 p-2 pb-0`
        Some(
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(px(6.0))
                .p_2()
                .when(!bottom_spacing, |el| el.pb_0())
                .children(card)
                .children(button)
                .into_any_element(),
        )
    }

    /// MonoCode `UpdateRailCard`: "Updated to X", What's new, and a close.
    fn render_updated_card(&self, version: String, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let open_version = version.clone();
        // `flex w-full items-start gap-2 rounded-lg px-2 py-2 pr-8
        // hover:bg-accent/10`
        let open = div()
            .id("update-card-open")
            .flex()
            .w_full()
            .items_start()
            .gap_2()
            .rounded(px(8.0))
            .px_2()
            .py_2()
            .pr_8()
            .cursor_pointer()
            .hover(move |s| s.bg(accent.opacity(0.1)))
            .on_click(
                cx.listener(move |this, _, _, cx| this.open_whats_new(open_version.clone(), cx)),
            )
            .child(
                // `mt-0.5 grid size-[18px] place-items-center`
                div()
                    .mt(px(2.0))
                    .size(px(18.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(img(APP_ICON.clone()).size(px(16.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child(format!("Updated to {version}")),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.5))
                            .child("What's new"),
                    ),
            );
        // `absolute right-1 top-1 size-6 rounded-md text-content/45
        // hover:bg-content/8 hover:text-content`
        let close = div()
            .id("update-card-dismiss")
            .absolute()
            .right(px(4.0))
            .top(px(4.0))
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(fg.opacity(0.45))
            .hover(move |s| s.bg(fg.opacity(0.08)).text_color(fg))
            // It sits over the row; without this the row's click opens
            // What's new too.
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| this.dismiss_installed_update(cx)))
            .child(Icon::new(IconName::X).size(IconSize::Sm));
        // `relative overflow-hidden rounded-lg bg-content/12`
        div()
            .relative()
            .overflow_hidden()
            .rounded(px(8.0))
            .bg(fg.opacity(0.12))
            .child(open)
            .child(close)
            .into_any_element()
    }

    /// MonoCode `SidebarUpdate`: "Update to X" (accent), "Downloading N%"
    /// while it runs, and BenCode's "Restart to update" for a restart put
    /// off while chats ran.
    fn render_update_button(&self, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let state = &self.updater;
        let busy = state.phase == Phase::Downloading;
        let label: SharedString = match state.phase {
            Phase::Downloading => match state.progress() {
                Some(percent) => format!("Downloading {percent}%").into(),
                None => "Downloading…".into(),
            },
            Phase::Ready => "Restart to update".into(),
            _ => format!("Update to {}", state.available_version().unwrap_or("?")).into(),
        };
        let icon = if busy {
            spinning_icon(
                "update-busy".into(),
                IconName::LoaderCircle,
                IconSize::Sm,
                fg.opacity(0.7),
            )
        } else {
            ExtraIcon::CircleArrowDown
                .icon()
                .size(IconSize::Sm)
                .color(accent)
                .into_any_element()
        };
        // `flex w-full items-center gap-2 rounded-lg px-2 py-2`
        div()
            .id("update-button")
            .flex()
            .w_full()
            .items_center()
            .gap_2()
            .rounded(px(8.0))
            .px_2()
            .py_2()
            .map(|el| {
                if busy {
                    // `bg-content/5 text-content/75 hover:bg-content/10`
                    el.bg(fg.opacity(0.05))
                        .text_color(fg.opacity(0.75))
                        .hover(move |s| s.bg(fg.opacity(0.1)).text_color(fg))
                } else {
                    // `bg-accent/15 text-content hover:bg-accent/20`
                    el.cursor_pointer()
                        .bg(accent.opacity(0.15))
                        .text_color(fg)
                        .hover(move |s| s.bg(accent.opacity(0.2)))
                        .on_click(cx.listener(|this, _, _, cx| this.install_update(cx)))
                }
            })
            .child(
                // `grid size-[18px] shrink-0 place-items-center`
                div()
                    .size(px(18.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(label),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .pl_2()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.4))
                            .child(format!("v{}", state.current_version())),
                    ),
            )
            .into_any_element()
    }
}
