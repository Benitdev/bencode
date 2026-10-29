//! MonoCode's git pill and git turns (`conversation/git_actions.rs`,
//! `transcript/git_turns.rs`): under a settled thread whose own edits are
//! uncommitted or unpushed, the next step (Stage, Commit, Commit & push,
//! Push); and on an action's message, the commits its turn made.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::menus::{Menu, MenuItem, SplitButton};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::git_actions::{GitAction, GitTurn};
use crate::db::Block;
use crate::ui::app_callback::app_callback;
use crate::ui::composer::cards::{CardIcon, card_frame, card_kind};
use crate::ui::tokens::{Ink, Text, Tone as _};
use crate::ui::transcript::selection::OwnsPress;

/// A commit's short id, as `git log --oneline` shows it.
const SHORT_SHA: usize = 7;

impl BenCodeApp {
    /// The pill's row in the trailer, while it shows.
    pub(super) fn render_git_pill(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.git_pill_shown(session_id) {
            return None;
        }
        let offers = self.git_offers(session_id);
        let current = *offers.first()?;
        let sid = session_id.to_string();
        let pill = if offers.len() == 1 {
            Button::new(
                SharedString::from(format!("git-pill-{session_id}")),
                current.label(),
            )
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Outline)
            .icon(current.icon())
            .on_click(cx.listener(move |this, _, _, cx| this.run_git_action(&sid, current, cx)))
            .into_any_element()
        } else {
            let menu = offers.iter().fold(Menu::new(), |menu, action| {
                let (sid, action) = (session_id.to_string(), *action);
                menu.item(
                    MenuItem::check(action.label(), action == current)
                        .icon(action.icon())
                        .on_click(app_callback(cx, move |this, cx| {
                            this.run_git_action(&sid, action, cx)
                        })),
                )
            });
            SplitButton::new(
                SharedString::from(format!("git-pill-{session_id}")),
                current.label(),
                menu,
            )
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Outline)
            .icon(current.icon())
            .on_click(cx.listener(move |this, _, _, cx| this.run_git_action(&sid, current, cx)))
            .into_any_element()
        };
        let fg = cx.theme().colors.fg;
        Some(
            div()
                .id(SharedString::from(format!("git-pill-row-{session_id}")))
                .w_full()
                .px_4()
                .pt_1()
                .pb_2()
                .flex()
                .items_center()
                .gap_2()
                .owns_press()
                .child(pill)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(current.description()),
                )
                .into_any_element(),
        )
    }
}

/// The card on a git action's message: what it asked for, then the
/// commits its turn made and where they were pushed.
pub fn git_turn_card(block: &Block, cx: &Context<BenCodeApp>) -> Option<AnyElement> {
    let action = GitAction::of_block(block)?;
    let turn = GitTurn::of_block(block);
    let fg = cx.theme().colors.fg;
    let mono_font = cx.theme().mono_family.clone();
    let web = turn.as_ref().and_then(|turn| turn.web.clone());
    let commits = turn.iter().flat_map(|turn| &turn.commits).map(|commit| {
        let short: String = commit.sha.chars().take(SHORT_SHA).collect();
        let url = web
            .as_ref()
            .map(|web| format!("{web}/commit/{}", commit.sha));
        let files = commit.files.len();
        div()
            .id(SharedString::from(format!(
                "git-commit-{}-{}",
                block.id, commit.sha
            )))
            .mt_1()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Secondary))
            .tooltip(Tooltip::text(if commit.files.is_empty() {
                commit.subject.clone()
            } else {
                commit.files.join("\n")
            }))
            .when_some(url, |el, url| {
                el.cursor_pointer()
                    .owns_press()
                    .hover(|s| s.text_color(fg.ink(Ink::Strong)))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
            .child(
                div()
                    .flex_none()
                    .font_family(mono_font.clone())
                    .text_color(fg.ink(Ink::Muted))
                    .child(short),
            )
            .child(div().min_w_0().truncate().child(commit.subject.clone()))
            .child(
                div()
                    .flex_none()
                    .text_color(fg.ink(Ink::Muted))
                    .child(format!(
                        "{files} {}",
                        if files == 1 { "file" } else { "files" }
                    )),
            )
    });
    let pushed = turn.as_ref().and_then(|turn| turn.pushed.clone());
    let nothing = turn
        .as_ref()
        .is_some_and(|turn| turn.commits.is_empty() && turn.pushed.is_none())
        && action != GitAction::Stage;
    Some(
        card_frame(&block.id, None, cx)
            .child(card_kind(
                CardIcon::Named(action.icon()),
                action.label().into(),
                cx,
            ))
            .children(commits)
            .children(pushed.map(|upstream| {
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(Icon::new(IconName::CloudUpload).size(IconSize::Xs))
                    .child(format!("Pushed to {upstream}"))
            }))
            .when(nothing, |el| {
                el.child(
                    div()
                        .mt_1()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child("No commit was made"),
                )
            })
            .into_any_element(),
    )
}
