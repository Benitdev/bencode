//! The editor toolbar's line blame (GitLens's blame hover): who last
//! changed the caret's line and when; resting on it opens a card with the
//! commit's message, its change to that line, and ways to open the commit
//! or the file's history.

use bencode_ui::buttons::{Button, ButtonVariant, IconButton};
use bencode_ui::data_display::Avatar;
use bencode_ui::overlays::HoverCard;
use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, AvatarSize, ControlSize, IconSize};
use gpui::{
    AnyElement, App, ClipboardItem, Context, FontWeight, IntoElement, ParentElement, SharedString,
    Styled, WeakEntity, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::blame::{LineBlame, Load};
use crate::git::blame::HunkLine;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// "September 29, 2026 11:34 AM" in the local time zone.
fn full_date(seconds: i64) -> String {
    match jiff::Timestamp::from_second(seconds) {
        Ok(at) => at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%B %-d, %Y %-I:%M %p")
            .to_string(),
        Err(_) => String::new(),
    }
}

impl BenCodeApp {
    /// The caret line's author and age, its card on hover; a click opens
    /// the commit.
    pub(super) fn blame_chip(&self, path: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let blame = self.caret_blame(path, cx)?;
        let fg = cx.theme().colors.fg;
        let label = if blame.commit.is_uncommitted() {
            "You, uncommitted".to_string()
        } else {
            let when =
                crate::ui::relative_time::since(blame.commit.time * 1000, crate::app::now_ms());
            format!("{}, {when}", blame.who())
        };
        let open = blame.clone();
        let trigger = div()
            .id("editor-blame-chip")
            .flex()
            .items_center()
            .gap_1()
            .max_w(px(220.0))
            .px_1p5()
            .py_0p5()
            .rounded(px(4.0))
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.fill(Fill::Hover)))
            .child(
                div()
                    .flex_none()
                    .child(Icon::new(IconName::GitCommitHorizontal).size(IconSize::Xs)),
            )
            .child(div().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !open.commit.is_uncommitted() {
                    this.open_blame_commit(
                        open.cwd.clone(),
                        open.commit.sha.clone(),
                        open.commit.summary.clone(),
                        cx,
                    );
                }
            }));
        let weak = cx.entity().downgrade();
        let path = path.to_string();
        Some(
            HoverCard::new("editor-blame-card", trigger, move |_, cx| {
                blame_card(&blame, &path, weak.clone(), cx)
            })
            .into_any_element(),
        )
    }
}

/// GitLens's blame hover: the author and when, the message, the commit's
/// actions and its change to the line.
fn blame_card(
    blame: &LineBlame,
    path: &str,
    app: WeakEntity<BenCodeApp>,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    let fg = colors.fg;
    let mono = theme.mono_family.clone();
    let commit = &blame.commit;
    let uncommitted = commit.is_uncommitted();
    let detail = match &blame.detail {
        Some(Load::Ready(detail)) => Some(detail.clone()),
        _ => None,
    };
    let message = if uncommitted {
        "Not committed yet".to_string()
    } else {
        detail
            .as_ref()
            .map(|d| d.message.clone())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| commit.summary.clone())
    };
    let when = if uncommitted {
        String::new()
    } else {
        format!(
            "{} ({})",
            crate::ui::relative_time::since(commit.time * 1000, crate::app::now_ms()),
            full_date(commit.time)
        )
    };
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Avatar::new(
                "editor-blame-avatar",
                SharedString::from(blame.who().to_string()),
            )
            .size(AvatarSize::Sm),
        )
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg.ink(Ink::Strong))
                .child(blame.who().to_string()),
        )
        .when(!when.is_empty(), |el| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(fg.ink(Ink::Muted))
                    .child(Icon::new(IconName::History).size(IconSize::Xs))
                    .child(when),
            )
        });
    let actions = (!uncommitted).then(|| {
        let (copy_sha, open_sha) = (commit.sha.clone(), commit.sha.clone());
        let (cwd, subject) = (blame.cwd.clone(), commit.summary.clone());
        let open_app = app.clone();
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("editor-blame-open", commit.short_sha().to_string())
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .icon(IconName::GitCommitHorizontal)
                    .on_click(move |_, _, cx| {
                        let opened = open_app.update(cx, |this, cx| {
                            this.open_blame_commit(
                                cwd.clone(),
                                open_sha.clone(),
                                subject.clone(),
                                cx,
                            )
                        });
                        if let Err(err) = opened {
                            log::debug!("blame card click after app drop: {err:#}");
                        }
                    }),
            )
            .child(
                IconButton::new("editor-blame-copy", IconName::Copy)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Copy commit id")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_sha.clone()))
                    }),
            )
    });
    let history_path = path.to_string();
    let history = Button::new("editor-blame-history", "File History")
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .icon(IconName::History)
        .on_click(move |_, _, cx| {
            let opened = app.update(cx, |this, cx| this.open_file_history(&history_path, cx));
            if let Err(err) = opened {
                log::debug!("blame card click after app drop: {err:#}");
            }
        });
    let hunk = detail
        .as_ref()
        .and_then(|detail| detail.hunk_at(blame.orig_line))
        .map(|(lines, cut)| {
            let diff = crate::ui::appearance::diff_colors(cx);
            let rows = lines.iter().map(|line| {
                let (mark, text, ink, wash) = match line {
                    HunkLine::Added(text) => ("+ ", text, diff.add_fg, diff.add_bg),
                    HunkLine::Removed(text) => ("- ", text, diff.del_fg, diff.del_bg),
                };
                div()
                    .px_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(ink)
                    .bg(wash)
                    .child(format!("{mark}{text}"))
            });
            div()
                .pt_2()
                .border_t_1()
                .border_color(colors.border)
                .font_family(mono.clone())
                .text_size(Text::Sm.px())
                .children(rows)
                .when(cut, |el| {
                    el.child(div().px_1().text_color(fg.ink(Ink::Faint)).child("…"))
                })
        });
    div()
        .w(px(440.0))
        .flex()
        .flex_col()
        .gap_2()
        .text_size(Text::Base.px())
        .child(header)
        .child(
            div()
                .text_color(fg.ink(Ink::Body))
                .whitespace_normal()
                .child(message),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .children(actions)
                .child(history),
        )
        .children(hunk)
        .into_any_element()
}
