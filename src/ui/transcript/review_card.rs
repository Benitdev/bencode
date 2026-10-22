//! MonoCode `SessionReview`: under a finished turn, the files the thread's
//! agent changed, with Undo, Keep and Review.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::session_review::{COLLAPSED_FILES, ReviewAction};
use crate::git::checkpoint::CheckpointFile;
use crate::ui::app_callback::app_callback;
use crate::ui::buttons::with_tooltip;
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::transcript::selection::OwnsPress;

/// MonoCode `text-amber-300/80`, "Mixed changes".
const MIXED: u32 = 0xfcd34d;

impl BenCodeApp {
    pub(super) fn render_session_review(&self, session_id: &str, cx: &Context<Self>) -> AnyElement {
        let Some(review) = self.checkpoints.reviews.get(session_id) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let diff = crate::ui::appearance::diff_colors(cx);
        let fg = colors.fg;
        let files = &review.files;
        let count = files.len();
        let (additions, deletions) = files
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let locked = self.session_undo_locked(session_id);
        let can_undo = !locked && files.iter().all(|f| f.undoable);
        let acting = review.acting;
        let shown = if review.expanded {
            count
        } else {
            count.min(COLLAPSED_FILES)
        };
        let hidden = count - shown;

        // Undo and Keep are quiet; Review is the outlined one (MonoCode).
        let button = |key: &str, label: &'static str, enabled: bool| {
            Button::new(
                SharedString::from(format!("review-{key}-{session_id}")),
                label,
            )
            .size(ControlSize::Sm)
            .variant(if key == "open" {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Ghost
            })
            .disabled(!enabled)
        };
        let tipped = |key: &str, tip: &'static str, button: Button| {
            with_tooltip(
                SharedString::from(format!("review-{key}-tip-{session_id}")),
                tip,
                button,
            )
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
                    .bg(fg.fill(Fill::Selected))
                    .child(
                        ExtraIcon::FileDiff
                            .icon()
                            .size(IconSize::Md)
                            .color(fg.ink(Ink::Secondary)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(Text::Base.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg.ink(Ink::Strong))
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
                            .text_size(Text::Sm.px())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().text_color(diff.add_fg).child(format!("+{additions}")))
                            .child(div().text_color(diff.del_fg).child(format!("-{deletions}"))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .child(tipped(
                        "undo",
                        undo_tip,
                        button("undo", "Undo", can_undo && !acting).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.checkpoints.confirm_undo = Some(undo_id.clone());
                                cx.notify();
                            },
                        )),
                    ))
                    .child(tipped(
                        "keep",
                        "Keep all session changes and dismiss this card",
                        button("keep", "Keep", !acting).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.resolve_session_review(&keep_id, ReviewAction::Keep, cx)
                            },
                        )),
                    ))
                    .child(tipped(
                        "open",
                        "Review changes",
                        button("open", "Review", true).on_click(cx.listener(
                            move |this, _, _, cx| this.open_session_changes(&review_id, None, cx),
                        )),
                    )),
            );

        let rows = files[..shown]
            .iter()
            .map(|file| self.render_review_file(session_id, file, cx));
        let list = div()
            .id(SharedString::from(format!("review-files-{session_id}")))
            .py_1()
            .border_t_1()
            .border_color(fg.fill(Fill::Line))
            .when(review.expanded, |el| {
                el.max_h(px(256.0)).overflow_y_scroll()
            })
            .children(rows);

        div()
            .w_full()
            .px_4()
            .pt_1()
            .pb_2()
            .child(
                div()
                    .overflow_hidden()
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(fg.fill(Fill::Strong))
                    .bg(fg.fill(Fill::Faint))
                    .child(header)
                    .children(review.error.clone().map(|error| {
                        div()
                            .px_3()
                            .pb_2()
                            .text_size(Text::Sm.px())
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
                                .border_color(fg.fill(Fill::Line))
                                .text_size(Text::Sm.px())
                                .text_color(fg.ink(Ink::Muted))
                                .cursor_pointer()
                                .owns_press()
                                .hover(|s| s.bg(fg.fill(Fill::Hover)).text_color(fg.ink(Ink::Body)))
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
                                    .color(fg.ink(Ink::Muted)),
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
        let diff = crate::ui::appearance::diff_colors(cx);
        let fg = colors.fg;
        let name = file.relative.rsplit('/').next().unwrap_or(&file.relative);
        let icon = crate::ui::file_tree::resolve_entry_icon(name, false, false);
        let (open_id, focus) = (session_id.to_string(), file.relative.clone());
        div()
            .id(SharedString::from(format!(
                "review-file-{session_id}-{}",
                file.relative
            )))
            .flex()
            .flex_none()
            .min_w_0()
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px_3()
            .text_color(fg.opacity(0.65))
            .cursor_pointer()
            .owns_press()
            .hover(|s| s.bg(fg.fill(Fill::Hover)).text_color(fg))
            .tooltip(Tooltip::text(file.relative.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_session_changes(&open_id, Some(focus.clone()), cx)
            }))
            .child(icon.size(IconSize::Sm))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_family.clone())
                    .text_size(Text::Base.px())
                    .child(file.relative.clone()),
            )
            .child(if file.exact {
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .text_size(Text::Sm.px())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .text_color(diff.add_fg)
                            .child(format!("+{}", file.additions)),
                    )
                    .child(
                        div()
                            .text_color(diff.del_fg)
                            .child(format!("-{}", file.deletions)),
                    )
            } else {
                div()
                    .flex_none()
                    .text_size(Text::Sm.px())
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
