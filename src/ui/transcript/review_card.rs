//! MonoCode `SessionReview`: under a finished turn, the files the thread's
//! agent changed, with Undo, Keep and Review.

use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::app::session_review::{COLLAPSED_FILES, ReviewAction};
use crate::git::checkpoint::CheckpointFile;
use crate::ui::app_callback::app_callback;
use crate::ui::icons::ExtraIcon;

/// MonoCode `text-amber-300/80`, "Mixed changes".
const MIXED: u32 = 0xfcd34d;

impl BenCodeApp {
    pub(super) fn render_session_review(&self, session_id: &str, cx: &Context<Self>) -> AnyElement {
        let Some(review) = self.checkpoints.reviews.get(session_id) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let files = &review.files;
        let count = files.len();
        let (additions, deletions) = files
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let locked = self.session_undo_locked(session_id);
        let can_undo = !locked && files.iter().all(|f| f.undoable);
        let acting = review.acting;
        let shown = if review.expanded { count } else { count.min(COLLAPSED_FILES) };
        let hidden = count - shown;

        // `h-7 rounded-md px-2.5 text-[11px] text-content/50
        // hover:bg-content/8 hover:text-content disabled:opacity-35`
        // Review is the filled one: `border border-content/12 bg-content/8
        // font-medium text-content/75 hover:bg-content/12`.
        let button = |key: &str, label: &'static str, tip: &'static str, enabled: bool| {
            let filled = key == "open";
            let (text, hover_bg) = if filled { (0.75, 0.12) } else { (0.5, 0.08) };
            div()
                .id(SharedString::from(format!("review-{key}-{session_id}")))
                .h(px(28.0))
                .px_2p5()
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(6.0))
                .text_size(px(11.0))
                .text_color(fg.opacity(text))
                .when(filled, |el| {
                    el.border_1()
                        .border_color(fg.opacity(0.12))
                        .bg(fg.opacity(0.08))
                        .font_weight(FontWeight::MEDIUM)
                })
                .tooltip(Tooltip::text(tip))
                .when(!enabled, |el| el.opacity(0.35))
                .when(enabled, |el| {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(fg.opacity(hover_bg)).text_color(fg))
                })
                .child(label)
        };
        let undo_tip = if can_undo {
            "Undo all session changes"
        } else if locked {
            "Undo is unavailable while another session is running in this project"
        } else {
            "Undo is unavailable because a file changed outside this session"
        };
        let (undo_id, keep_id, review_id, toggle_id) = (
            session_id.to_string(),
            session_id.to_string(),
            session_id.to_string(),
            session_id.to_string(),
        );

        let header = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2p5()
            .px_3()
            .py_2p5()
            .child(
                div()
                    .size(px(32.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(fg.opacity(0.08))
                    .child(ExtraIcon::FileDiff.render(px(16.0), fg.opacity(0.55))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg.opacity(0.8))
                            .child(format!(
                                "Changed {count} {}",
                                if count == 1 { "file" } else { "files" }
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().text_color(colors.success).child(format!("+{additions}")))
                            .child(div().text_color(colors.danger).child(format!("-{deletions}"))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .child(
                        button("undo", "Undo", undo_tip, can_undo && !acting).when(
                            can_undo && !acting,
                            |el| {
                                el.on_click(cx.listener(move |this, _, _, cx| {
                                    this.checkpoints.confirm_undo = Some(undo_id.clone());
                                    cx.notify();
                                }))
                            },
                        ),
                    )
                    .child(
                        button(
                            "keep",
                            "Keep",
                            "Keep all session changes and dismiss this card",
                            !acting,
                        )
                        .when(!acting, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| {
                                this.resolve_session_review(&keep_id, ReviewAction::Keep, cx)
                            }))
                        }),
                    )
                    .child(
                        button("open", "Review", "Review changes", true).on_click(cx.listener(move |this, _, _, cx| {
                                this.open_session_changes(&review_id, None, cx)
                            })),
                    ),
            );

        let rows = files[..shown]
            .iter()
            .map(|file| self.render_review_file(session_id, file, cx));
        let list = div()
            .id(SharedString::from(format!("review-files-{session_id}")))
            .flex()
            .flex_col()
            .py_1()
            .border_t_1()
            .border_color(fg.opacity(0.07))
            .when(review.expanded, |el| el.max_h(px(256.0)).overflow_y_scroll())
            .children(rows);

        div()
            .w_full()
            .px_4()
            .pt_1()
            .pb_2()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(fg.opacity(0.12))
                    .bg(fg.opacity(0.03))
                    .child(header)
                    .children(review.error.clone().map(|error| {
                        div()
                            .px_3()
                            .pb_2()
                            .text_size(px(11.0))
                            .text_color(colors.danger)
                            .child(error)
                    }))
                    .child(list)
                    .when(count > COLLAPSED_FILES, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("review-more-{session_id}")))
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .h(px(32.0))
                                .px_3()
                                .border_t_1()
                                .border_color(fg.opacity(0.07))
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.45))
                                .cursor_pointer()
                                .hover(|s| s.bg(fg.opacity(0.05)).text_color(fg.opacity(0.7)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_session_review(&toggle_id, cx)
                                }))
                                .child(
                                    Icon::new(if review.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.45)),
                                )
                                .child(if review.expanded {
                                    "Show fewer files".to_string()
                                } else {
                                    format!(
                                        "Show {hidden} more {}",
                                        if hidden == 1 { "file" } else { "files" }
                                    )
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// MonoCode `FileRow`: opens the review at this file.
    fn render_review_file(
        &self,
        session_id: &str,
        file: &CheckpointFile,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let name = file.relative.rsplit('/').next().unwrap_or(&file.relative);
        let (icon, tint) = crate::ui::file_tree::resolve_entry_icon(name, false, false);
        let (open_id, focus) = (session_id.to_string(), file.relative.clone());
        div()
            .id(SharedString::from(format!("review-file-{session_id}-{}", file.relative)))
            .flex()
            .flex_none()
            .min_w_0()
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px_3()
            .text_color(fg.opacity(0.65))
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.05)).text_color(fg))
            .tooltip(Tooltip::text(file.relative.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_session_changes(&open_id, Some(focus.clone()), cx)
            }))
            .child(Icon::new(icon).size(IconSize::Sm).color(tint))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_family.clone())
                    .text_size(px(12.0))
                    .child(file.relative.clone()),
            )
            .child(if file.exact {
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().text_color(colors.success).child(format!("+{}", file.additions)))
                    .child(div().text_color(colors.danger).child(format!("-{}", file.deletions)))
            } else {
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(gpui::rgb(MIXED).opacity(0.8))
                    .child("Mixed changes")
            })
    }

    /// Undo rewrites files, so it asks first.
    pub(crate) fn render_session_undo_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session_id = self.checkpoints.confirm_undo.clone()?;
        let count = self
            .checkpoints
            .reviews
            .get(&session_id)
            .map_or(0, |review| review.files.len());
        let message = format!(
            "{count} {} will go back to what {} held before this session edited {}.",
            if count == 1 { "file" } else { "files" },
            if count == 1 { "it" } else { "they" },
            if count == 1 { "it" } else { "them" },
        );
        let cancel = app_callback(cx, |this, cx| {
            this.checkpoints.confirm_undo = None;
            cx.notify();
        });
        let undo = app_callback(cx, move |this, cx| {
            this.resolve_session_review(&session_id, ReviewAction::Undo, cx)
        });
        Some(
            ConfirmDialog::new("session-undo", "Undo session changes?", message, cancel)
                .confirm("Undo")
                .destructive()
                .on_confirm(undo)
                .into_any_element(),
        )
    }
}
