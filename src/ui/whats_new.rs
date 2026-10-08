//! MonoCode `WhatsNewDialog`: a version's CHANGELOG section
//! (`app/release_notes.rs`) in a dialog, from the "Updated to" card and
//! Settings › About.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::release_notes;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::transcript::markdown::AgentMarkdown;

/// MonoCode `h-[min(72vh,640px)]`, less the dialog's title and buttons.
const NOTES_HEIGHT: f32 = 480.0;

impl BenCodeApp {
    pub(crate) fn render_whats_new(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let version = self.updater.whats_new.clone()?;
        let notes = release_notes::notes_for(&version);
        let detail = match notes.as_ref().and_then(|notes| notes.date.as_deref()) {
            Some(date) => format!("BenCode {version} · {date}"),
            None => format!("BenCode {version}"),
        };
        let body = match notes {
            Some(notes) if !notes.markdown.is_empty() => {
                AgentMarkdown::new("whats-new-notes", notes.markdown).into_any_element()
            }
            _ => div()
                .text_size(px(13.0))
                .text_color(cx.theme().colors.fg.opacity(0.6))
                .child("Release notes for this version are not available in this build.")
                .into_any_element(),
        };
        let close = app_callback(cx, |this, cx| this.close_whats_new(cx));
        Some(
            Dialog::new("whats-new", "What's new", close)
                .detail(detail)
                .child(
                    div()
                        .id("whats-new-scroll")
                        .max_h(px(NOTES_HEIGHT))
                        .overflow_y_scroll()
                        .child(body),
                )
                .action(move |close| {
                    Button::new("whats-new-close", "Close")
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .into_any_element(),
        )
    }
}
