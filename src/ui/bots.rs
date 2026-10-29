//! A Bot in the main area (MonoCode `App.tsx`'s `monoCovers`,
//! `MonoHeader`, `ConfirmReset`): its conversation in place of the
//! workspace, its tabs and the inspector, under its title bar
//! (`ui/titlebar/bot.rs`), with its Overview beside the chat
//! (`ui/bot_overview/`), the greeting that fills the chat before the first
//! message, and the confirmations its settings (`ui/bot_settings.rs`) and
//! Settings › Bots ask for. The model is the composer's
//! (`ui/composer/bot.rs`).

use bencode_ui::feedback::EmptyState;
use bencode_ui::overlays::ConfirmDialog;
use bencode_ui::primitives::IconName;
use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::bots::{Bot, projects_phrase};
use crate::ui::app_callback::app_callback;
use crate::ui::figure::figure_view;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    /// MonoCode `monoCovers`: a Bot's conversation is what the main area
    /// shows (a view opened over it still wins).
    pub fn bot_covers(&self) -> bool {
        self.surface.is_none() && self.bots.view.is_some()
    }

    /// The Bot's column: its title bar, then the chat and details. It
    /// covers the workspace's status, as MonoCode's covers its footer.
    pub(crate) fn render_bot_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.bot_covers() {
            return None;
        }
        let bot = self.bot_in_view()?.clone();
        let state = self.bot_state(&bot);
        let glass = self.glass(cx);
        let bg = cx.theme().colors.bg;
        let chat = match self.bot_view_session().map(str::to_string) {
            Some(session_id) => self.render_session_pane(&session_id, false, cx),
            None => EmptyState::new(
                "bot-loading",
                IconName::MessageSquare,
                "Opening the conversation…",
            )
            .into_any_element(),
        };
        // MonoCode `monoShownSessionView`: the session it started, as a
        // pane of its own over the Overview.
        let shown = self
            .bot_shown_session()
            .map(str::to_string)
            .map(|id| (id.clone(), self.render_session_pane(&id, false, cx)));
        let resizing = self.bots.overview.resizing.is_some();
        Some(
            div()
                .id("bot-view")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(glass.body(bg))
                // MonoCode `useDragResize`: the drag follows the pointer
                // anywhere over the view.
                .when(resizing, |el| {
                    el.cursor_col_resize()
                        .on_mouse_move(cx.listener(
                            |this, event: &gpui::MouseMoveEvent, window, cx| {
                                if let Some((start_x, start)) = this.bots.overview.resizing {
                                    let width = crate::app::bot_overview::resized_width(
                                        start_x,
                                        start,
                                        crate::ui::scale::logical(event.position.x),
                                        crate::ui::scale::logical(window.viewport_size().width),
                                    );
                                    if width != this.bots.overview.width {
                                        this.bots.overview.width = width;
                                        cx.notify();
                                    }
                                }
                            },
                        ))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_bot_overview_resize(cx)),
                        )
                        .on_mouse_up_out(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_bot_overview_resize(cx)),
                        )
                })
                .child(self.render_bot_titlebar(&bot, &state, cx))
                // The plan's usage is under the composer instead.
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex_1().min_w_0().h_full().child(chat))
                        .when(self.bots.overview_open, |el| {
                            el.child(self.render_bot_overview(&bot, &state, shown, cx))
                        }),
                )
                .into_any_element(),
        )
    }

    /// The Bot's delete and reset confirmations. Full-window dialogs: in
    /// `BenCodeApp::render`'s `dialogs` list.
    pub(crate) fn render_bot_dialogs(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if let Some(id) = self.bots.pending_delete.clone() {
            let name = self.bot(&id)?.display_name();
            let close = app_callback(cx, |this, cx| {
                this.bots.pending_delete = None;
                cx.notify();
            });
            let delete = app_callback(cx, move |this, cx| this.delete_bot(&id, cx));
            return Some(
                ConfirmDialog::new(
                    "bot-delete-confirm",
                    format!("Delete {name}?"),
                    "Its conversation is deleted with it. Its soul and memory stay on disk.",
                    close,
                )
                .confirm("Delete")
                .destructive()
                .on_confirm(delete)
                .into_any_element(),
            );
        }
        if let Some(id) = self.bots.pending_defaults.clone() {
            let bot = self.bot(&id)?;
            let name = bot.display_name();
            let default = crate::bots::default_name(&bot.id);
            let close = app_callback(cx, |this, cx| {
                this.bots.pending_defaults = None;
                cx.notify();
            });
            let reset = app_callback(cx, move |this, cx| this.reset_bot_defaults(&id, cx));
            return Some(
                ConfirmDialog::new(
                    "bot-defaults-confirm",
                    format!("Reset {name} to its defaults?"),
                    format!(
                        "Its soul goes back to the default and its name to {default}. Changes \
                         to its soul can't be recovered. Its conversation, projects, memory \
                         and habits will be kept."
                    ),
                    close,
                )
                .confirm("Reset Bot")
                .destructive()
                .on_confirm(reset)
                .into_any_element(),
            );
        }
        if !self.bots.pending_reset {
            return None;
        }
        let name = self.bot_in_view()?.display_name();
        let close = app_callback(cx, |this, cx| {
            this.bots.pending_reset = false;
            cx.notify();
        });
        let reset = app_callback(cx, |this, cx| this.reset_bot_session(cx));
        Some(
            ConfirmDialog::new(
                "bot-reset-confirm",
                format!("Reset {name}'s conversation?"),
                "All messages in this bot's conversation will be deleted and any active reply \
                 will be stopped. This can't be undone. Its soul, memory and habits will be \
                 kept.",
                close,
            )
            .confirm("Reset conversation")
            .destructive()
            .on_confirm(reset)
            .into_any_element(),
        )
    }

    /// MonoCode `MonoHeader` with `greeting`: before the first message the
    /// header fills the chat (`min-h-full justify-center py-12`), its
    /// figure at `size-14`, breathing, standing on its name pill (`-mt-2`,
    /// drawn over it) and saying hello.
    pub(crate) fn render_bot_greeting(&self, bot: &Bot, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let names: Vec<String> = self
            .bot_projects(bot)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        // `.mono-pill`: the text colour at 14% and 6%; white in light mode,
        // lifted by a shadow.
        let pill = div()
            .mt(px(-8.0))
            .flex()
            .h(px(32.0))
            .items_center()
            .rounded_full()
            .border_1()
            .px(px(14.0))
            .map(|el| {
                if theme.is_dark() {
                    el.border_color(fg.opacity(0.14)).bg(fg.fill(Fill::Hover))
                } else {
                    el.border_color(gpui::black().opacity(0.06))
                        .bg(gpui::white())
                        .shadow_md()
                }
            })
            .text_size(Text::Md.px())
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(bot.display_name());
        div()
            .id("bot-greeting")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .min_h_full()
                    .px_6()
                    .py_12()
                    .text_center()
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(div().h(px(56.0)))
                            .child(pill)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .flex()
                                    .justify_center()
                                    .child(figure_view(
                                        bot.figure(),
                                        56.0,
                                        self.bot_color(bot),
                                        self.bot_look(bot).lively(),
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        // `mt-3 max-w-sm text-[13px] leading-relaxed text-content/50`
                        div()
                            .mt_3()
                            .max_w(px(384.0))
                            .text_size(Text::Md.px())
                            .line_height(px(21.0))
                            .text_color(fg.ink(Ink::Secondary))
                            .child(greeting_text(&names)),
                    ),
            )
            .into_any_element()
    }
}

/// Its hello (MonoCode `MonoHeader`'s, in BenCode's words), naming its
/// projects with `projects_phrase`.
fn greeting_text(projects: &[String]) -> String {
    if projects.is_empty() {
        "Give me projects to look after in my settings, or just ask me something.".to_string()
    } else {
        format!(
            "I look after {}: their threads, notes and folders. Ask how things stand, or hand \
             me work to run.",
            projects_phrase(projects)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeting_names_its_projects() {
        assert_eq!(
            greeting_text(&[]),
            "Give me projects to look after in my settings, or just ask me something."
        );
        assert_eq!(
            greeting_text(&["bencode".into(), "site".into()]),
            "I look after bencode and site: their threads, notes and folders. Ask how things \
             stand, or hand me work to run."
        );
    }
}
