//! MonoCode `InboxDetail`: identity, title, people and times, the actions
//! (Send to agent with its project, pull request actions, Open in GitHub),
//! labels, the body, then a pull request's checks and the conversation.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use crate::ui::scale::px;

use super::comments::{author_name, byline, byline_dot, comment_card, review_tint};
use super::{kind_label, project_name, relative_time, status_mark};
use crate::app::BenCodeApp;
use crate::github::Kind;
use crate::ui::app_callback::app_callback;
use crate::ui::composer::inbox_card::label_chip;

impl BenCodeApp {
    pub(super) fn render_inbox_detail(&self, cx: &Context<Self>) -> AnyElement {
        let Some(item) = self
            .inbox
            .selected
            .as_deref()
            .and_then(|key| self.inbox.item(key))
        else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(EmptyState::new(
                    "inbox-none",
                    IconName::Inbox,
                    "Select an inbox item",
                ))
                .into_any_element();
        };
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let now = crate::app::now_ms();
        let details = self.inbox.details.get(&key);
        let loaded = details.and_then(|d| d.as_ref().ok());
        let author = loaded.map(|d| d.author.clone()).filter(|a| !a.is_empty());
        let (base, head) = loaded
            .map(|d| (d.base_ref.clone(), d.head_ref.clone()))
            .unwrap_or_default();
        let url = item.url.clone();

        // State, number and repository, with Open in GitHub at the far end.
        let identity = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.0))
            .text_color(fg.opacity(0.5))
            .child(state_pill(item, cx))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(format!("#{} · {}", item.number, item.repo)),
            )
            .child(div().flex_1())
            .child(
                IconButton::new("inbox-open", IconName::ExternalLink)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Open in GitHub")
                    .disabled(url.is_empty())
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );

        let dot = || byline_dot(fg);
        let mut meta = byline(fg);
        if let Some(author) = author.clone() {
            meta = meta.child(author_name(author, fg));
        }
        let created = relative_time(&item.created_at, now);
        if !created.is_empty() {
            meta = meta.child(format!("opened {created}")).child(dot());
        }
        meta = meta.child(format!("updated {}", relative_time(&item.updated_at, now)));
        if !item.assignees.is_empty() {
            meta = meta
                .child(dot())
                .child(format!("assigned to {}", item.assignees.join(", ")));
        }
        let decision = loaded.map_or("", |d| d.review_decision.as_str());
        let review = match decision {
            "APPROVED" => "Approved",
            "CHANGES_REQUESTED" => "Changes requested",
            "REVIEW_REQUIRED" => "Review required",
            _ => "",
        };
        if !review.is_empty() {
            let color = review_tint(decision, colors).unwrap_or(fg.opacity(0.6));
            meta = meta.child(dot()).child(div().text_color(color).child(review));
        }

        let branches = (!base.is_empty() && !head.is_empty()).then(|| {
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(branch_chip(&head, cx))
                .child(
                    Icon::new(IconName::ArrowRight)
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.4)),
                )
                .child(branch_chip(&base, cx))
        });

        let start_key = key.clone();
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            // MonoCode starts threads from issues only.
            .when(item.kind == Kind::Issue, |el| {
                el.child(
                    Button::new("inbox-start", "Send to agent")
                        .primary()
                        .icon(IconName::Sparkles)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_inbox_item(&start_key, cx)
                        })),
                )
                .child(self.render_inbox_project_picker(item, cx))
            })
            .when(item.kind == Kind::Pr, |el| {
                el.children(self.render_pr_actions(item, &base, &head, cx))
            });

        let body = match details {
            None => div()
                .text_size(px(13.0))
                .text_color(fg.opacity(0.45))
                .child("Loading…")
                .into_any_element(),
            Some(Err(err)) => div()
                .text_size(px(12.0))
                .text_color(colors.danger.opacity(0.9))
                .child(SharedString::from(err.clone()))
                .into_any_element(),
            Some(Ok(d)) if d.body.trim().is_empty() => div()
                .text_size(px(13.0))
                .italic()
                .text_color(fg.opacity(0.45))
                .child("No description provided.")
                .into_any_element(),
            Some(Ok(d)) => MarkdownRenderer::new(
                SharedString::from(format!("inbox-body-{key}")),
                d.body.clone(),
            )
            .into_any_element(),
        };
        // The description reads as the opening comment, in the same card.
        let description = comment_card(fg)
            .mt_2()
            .child(
                byline(fg)
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .when_some(author, |el, author| el.child(author_name(author, fg)))
                    .child("Description"),
            )
            .child(div().px_4().py_3().text_size(px(14.0)).child(body));

        let detail = div()
            .id("inbox-detail")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(1024.0))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px_8()
                    .pt_4()
                    .pb_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(identity)
                            .child(
                                div()
                                    .text_size(px(22.0))
                                    .line_height(px(28.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg)
                                    .line_clamp(3)
                                    .child(SharedString::from(item.title.clone())),
                            )
                            .child(meta)
                            .children(branches)
                            .when(!item.labels.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .children(item.labels.iter().map(|l| label_chip(l, cx))),
                                )
                            }),
                    )
                    .child(actions)
                    .children(self.render_pr_action_status(cx))
                    .child(description)
                    .when(item.kind == Kind::Pr, |el| {
                        el.child(self.render_pr_checks(item, cx))
                    })
                    .child(self.render_inbox_conversation(item, cx)),
            );
        crate::ui::scrollbar::Scrolled::new("inbox-detail-scrollbar", detail).into_any_element()
    }

    /// MonoCode `InboxProjectPicker`: which rail project the agent works in.
    fn render_inbox_project_picker(
        &self,
        item: &crate::github::WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let chosen = self.inbox_start_project(item);
        let key = item.key();
        let menu = self
            .inbox_projects()
            .into_iter()
            .fold(Menu::new(), |menu, path| {
                let (key, target) = (key.clone(), path.clone());
                menu.item(
                    MenuItem::radio(
                        project_name(&path),
                        crate::app::same_project_path(&path, &chosen),
                    )
                    .on_click(app_callback(cx, move |this, cx| {
                        this.inbox.start_project.insert(key.clone(), target.clone());
                        cx.notify();
                    })),
                )
            });
        DropdownMenu::new("inbox-start-project", project_name(&chosen), menu)
            .variant(ButtonVariant::Secondary)
            .icon(IconName::Folder)
    }
}

/// The item's state as GitHub shows it: icon and word on a tinted pill.
fn state_pill(item: &crate::github::WorkItem, cx: &gpui::App) -> impl IntoElement {
    let (icon, tint, status) = status_mark(item, cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(tint.opacity(0.14))
        .text_color(tint)
        .font_weight(FontWeight::MEDIUM)
        .child(Icon::new(icon).size(IconSize::Xs).color(tint))
        .child(format!("{status} {}", kind_label(item.kind).to_lowercase()))
}

/// A branch name in code type, cut short when the row is narrow.
fn branch_chip(name: &str, cx: &gpui::App) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    div()
        .min_w_0()
        .max_w(px(360.0))
        .truncate()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(fg.opacity(0.07))
        .font_family(cx.theme().mono_family.clone())
        .text_size(px(11.5))
        .text_color(fg.opacity(0.7))
        .child(SharedString::from(name.to_string()))
}
