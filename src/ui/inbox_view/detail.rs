//! MonoCode `InboxDetail`: identity, title, people and times, the actions
//! (Send to agent with its project, pull request actions, Open in GitHub),
//! labels, the body, then a pull request's checks and the conversation.

use ely_gpui_component::buttons::{Button, ButtonVariant};
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
        let (icon, tint, status) = status_mark(item, cx);
        let details = self.inbox.details.get(&key);
        let loaded = details.and_then(|d| d.as_ref().ok());
        let dot = || div().text_color(fg.opacity(0.35)).child("·");
        let mut meta = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .text_size(px(12.0))
            .text_color(fg.opacity(0.5));
        if let Some(author) = loaded.map(|d| d.author.clone()).filter(|a| !a.is_empty()) {
            meta = meta.child(author).child(dot());
        }
        if !item.assignees.is_empty() {
            meta = meta.child(item.assignees.join(", ")).child(dot());
        }
        let created = relative_time(&item.created_at, now);
        if !created.is_empty() {
            meta = meta.child(format!("Created {created}")).child(dot());
        }
        meta = meta.child(format!("Updated {}", relative_time(&item.updated_at, now)));
        let (base, head) = loaded
            .map(|d| (d.base_ref.clone(), d.head_ref.clone()))
            .unwrap_or_default();
        if !base.is_empty() && !head.is_empty() {
            meta = meta.child(dot()).child(format!("{base} ← {head}"));
        }
        if let Some((review, color)) = loaded.and_then(|d| match d.review_decision.as_str() {
            "APPROVED" => Some(("Approved", colors.success)),
            "CHANGES_REQUESTED" => Some(("Changes requested", colors.danger)),
            "REVIEW_REQUIRED" => Some(("Review required", fg.opacity(0.5))),
            _ => None,
        }) {
            meta = meta
                .child(dot())
                .child(div().text_color(color).child(review));
        }
        let start_key = key.clone();
        let url = item.url.clone();
        let actions =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .pt_0p5()
                // MonoCode starts threads from issues only.
                .when(item.kind == Kind::Issue, |el| {
                    el.child(
                        Button::new("inbox-start", "Send to agent")
                            .primary()
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_inbox_item(&start_key, cx)
                            })),
                    )
                    .child(self.render_inbox_project_picker(item, cx))
                })
                .when(item.kind == Kind::Pr, |el| {
                    el.children(self.render_pr_actions(item, &base, &head, cx))
                })
                .child(
                    Button::new("inbox-open", "Open in GitHub")
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .icon(IconName::ExternalLink)
                        .disabled(url.is_empty())
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
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
                .text_color(fg.opacity(0.45))
                .child("No description provided.")
                .into_any_element(),
            Some(Ok(d)) => MarkdownRenderer::new(
                SharedString::from(format!("inbox-body-{key}")),
                d.body.clone(),
            )
            .into_any_element(),
        };
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
                    .gap(px(10.0))
                    .px_8()
                    .pt_5()
                    .pb_8()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.5))
                            .child(Icon::new(icon).size(IconSize::Xs).color(tint))
                            .child(format!(
                                "{status} {} · #{} · {}",
                                kind_label(item.kind).to_lowercase(),
                                item.number,
                                item.repo
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(20.0))
                            .line_height(px(25.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .line_clamp(2)
                            .child(SharedString::from(item.title.clone())),
                    )
                    .child(meta)
                    .child(actions)
                    .children(self.render_pr_action_status(cx))
                    .when(!item.labels.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .children(item.labels.iter().map(|l| label_chip(l, cx))),
                        )
                    })
                    .child(
                        div()
                            .mt_2()
                            .pt_4()
                            .border_t_1()
                            .border_color(colors.border)
                            .text_size(px(14.0))
                            .child(body),
                    )
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
