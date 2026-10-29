//! The title bar over a Bot's view (MonoCode `TitleBar.tsx` with `mono`:
//! `MonoTitle` and its trailing controls): its figure, moving with its
//! mood, its name and the status in words (in the accent while it needs
//! you, a "!" over its figure then; a failed last turn, or the habit it
//! sleeps until), then the Overview pill and the gear, "{name} settings",
//! which opens its settings dialog.
//!
//! MonoCode's header has no floating-chat button; BenCode opens the float
//! from the Bot's rail menu, as MonoCode does.

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::app::BenCodeApp;
use crate::bots::{Bot, BotState, BotStatus};
use crate::ui::figure::figure_view;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::window_drag::claim_press;

impl BenCodeApp {
    /// MonoCode `TitleBar` in a Bot's view: `h-10 border-b border-stroke`,
    /// laid out as the views' title bar is, so the traffic lights and Back
    /// have the same room.
    pub(crate) fn render_bot_titlebar(
        &self,
        bot: &Bot,
        state: &BotState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let compact = self.compact_rail_active();
        let beside_rail = self.is_rail_open || compact;
        self.window_drag_region(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .h(px(crate::ui::sidebar::TITLEBAR_HEIGHT))
            .border_b_1()
            .border_color(colors.border)
            .when(!self.is_rail_open && cfg!(target_os = "macos"), |el| {
                // MonoCode `pl-[22px]` beside the compact rail.
                let gap = if compact { 22.0 } else { 72.0 };
                el.child(div().flex_none().w(gpui::px(gap)))
            })
            .when(!beside_rail, |el| {
                el.child(
                    claim_press(div())
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(
                            IconButton::new("bot-back", IconName::ChevronLeft)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Back (⌘[)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_bot_view(cx);
                                })),
                        )
                        .child(
                            IconButton::new("bot-toggle-projects", IconName::PanelLeft)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Show Navigator (⌘B)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.is_rail_open = true;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(self.render_bot_title(bot, state, cx))
            .child(self.render_bot_title_controls(bot, cx))
    }

    /// MonoCode `MonoTitle`: `flex min-w-0 flex-1 items-center gap-2 px-4`,
    /// the mascot at `size-4`, the name in `text-[13px] font-medium`, the
    /// status label in `text-[12px]`.
    fn render_bot_title(
        &self,
        bot: &Bot,
        state: &BotState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let look = self.bot_look(bot);
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_4()
            .child(figure_view(
                bot.figure(),
                16.0,
                self.bot_color(bot),
                look,
                cx,
            ))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Md.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.fg)
                    .child(bot.display_name()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(Text::Base.px())
                    .text_color(if state.status == BotStatus::NeedsYou {
                        colors.accent
                    } else {
                        colors.fg.ink(Ink::Muted)
                    })
                    .child(self.bot_status_label(bot, look)),
            )
    }

    /// MonoCode's trailing Bot controls (`flex items-center gap-1 px-3`):
    /// the Overview pill, `h-6 rounded-md px-2 text-[12px]` with its icon,
    /// lit while on, and the gear.
    fn render_bot_title_controls(&self, bot: &Bot, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let overview = self.bots.overview_open;
        let ink = if overview { fg } else { fg.ink(Ink::Body) };
        claim_press(div())
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_3()
            .child(
                div()
                    .id("bot-overview-toggle")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h(px(24.0))
                    .px_2()
                    .rounded(px(6.0))
                    .text_size(Text::Base.px())
                    .cursor_pointer()
                    .text_color(ink)
                    .bg(fg.opacity(if overview { 0.10 } else { 0.05 }))
                    .when(!overview, |el| {
                        el.hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                    })
                    .tooltip(Tooltip::text(if overview {
                        "Hide overview"
                    } else {
                        "Show overview"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_bot_overview(cx)))
                    .child(
                        Icon::new(IconName::LayoutDashboard)
                            .size(IconSize::Xs)
                            .color(ink),
                    )
                    .child("Overview"),
            )
            .child(
                IconButton::new("bot-settings", IconName::Settings)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(SharedString::from(format!(
                        "{} settings",
                        bot.display_name()
                    )))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_bot_settings(crate::app::bots::SettingsSection::General, cx)
                    })),
            )
    }
}
