//! A bot's digest over its composer (`bots/digest.rs`), in BenCode's window
//! and the floating chat alike: its projects' threads that wait on the user
//! (Allow / Deny in place, Answer) or finished unseen, a row each that opens
//! the thread, and × to set them aside.

use std::rc::Rc;

use bencode_ui::buttons::{Button, ButtonVariant, IconButton};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement, SharedString, Styled, Window, div,
};

use crate::app::BenCodeApp;
use crate::bots::digest::{SHOWN, Standing};
use crate::bots::{Bot, projects_phrase};
use crate::ui::rows::ListRow;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// What a row's click does: opens its thread, from whichever window.
pub type OpenThread = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// "1 waiting · 2 finished".
fn counts(waiting: usize, finished: usize) -> String {
    let parts: Vec<String> = [(waiting, "waiting"), (finished, "finished")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    parts.join(" · ")
}

impl BenCodeApp {
    /// The digest card for `bot`, or nothing while its projects have none.
    pub(crate) fn render_bot_digest(
        &self,
        bot: &Bot,
        open: OpenThread,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let items = self.bot_digest(bot);
        if items.is_empty() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let finished = items
            .iter()
            .filter(|item| item.standing == Standing::Finished)
            .count();
        let waiting = items.len() - finished;
        let names: Vec<String> = self
            .bot_projects(bot)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let place = if names.is_empty() {
            "your projects".to_string()
        } else {
            projects_phrase(&names)
        };
        let bot_id = bot.id.clone();
        let close = IconButton::new("bot-digest-close", IconName::X)
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .tooltip("Set these aside")
            .on_click(cx.listener(move |this, _, _, cx| this.set_bot_digest_aside(&bot_id, cx)));
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(10.0))
            .pr_1()
            .pt_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(format!("In {place} · {}", counts(waiting, finished))),
            )
            .child(close);
        let rows = items.iter().take(SHOWN).map(|item| {
            let project = self.rail_project_label(&item.cwd);
            let line = match &item.standing {
                Standing::Approval(what) if what.is_empty() => {
                    format!("{project} · Needs approval")
                }
                Standing::Approval(what) => format!("{project} · Allow {what}?"),
                Standing::Question => format!("{project} · Has a question"),
                Standing::Finished => format!("{project} · Finished"),
            };
            let id = item.session_id.clone();
            let button = |kind: &str, label: &'static str, primary: bool| {
                Button::new(SharedString::from(format!("bot-digest-{kind}-{id}")), label)
                    .size(ControlSize::Sm)
                    .variant(if primary {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Subtle
                    })
            };
            // A button keeps its press: the row's own click opens the thread.
            let trailing: AnyElement = match &item.standing {
                Standing::Approval(_) => {
                    let (allow, deny) = (id.clone(), id.clone());
                    div()
                        .flex()
                        .gap_1()
                        .child(button("allow", "Allow", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.answer_permission(&allow, true, cx);
                            },
                        )))
                        .child(button("deny", "Deny", false).on_click(cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.answer_permission(&deny, false, cx);
                            },
                        )))
                        .into_any_element()
                }
                Standing::Question => {
                    let (open, answer) = (open.clone(), id.clone());
                    button("answer", "Answer", true)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            open(&answer, window, cx);
                        })
                        .into_any_element()
                }
                Standing::Finished => div().into_any_element(),
            };
            let (open, opened) = (open.clone(), id.clone());
            ListRow::new(format!("bot-digest-{id}"), item.title.clone())
                .tile(crate::ui::rail::widgets::figure_icon(
                    self.project_figure(&item.cwd),
                    self.project_color(&item.cwd),
                    false,
                    24.0,
                    cx,
                ))
                .line(line)
                .tooltip("Open thread")
                .on_click(move |_, window, cx| open(&opened, window, cx))
                .trailing(trailing)
        });
        let more = items.len().saturating_sub(SHOWN);
        Some(
            div()
                .mx(px(6.0))
                .mb_2()
                .p_1()
                .rounded(px(8.0))
                .border_1()
                .border_color(fg.fill(Fill::Strong))
                .bg(fg.fill(Fill::Faint))
                .child(header)
                .children(rows)
                .children((more > 0).then(|| {
                    div()
                        .px(px(10.0))
                        .pb_1()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(format!("{more} more in Agents"))
                }))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_counts_only_what_there_is() {
        assert_eq!(counts(1, 2), "1 waiting · 2 finished");
        assert_eq!(counts(0, 3), "3 finished");
        assert_eq!(counts(2, 0), "2 waiting");
    }
}
