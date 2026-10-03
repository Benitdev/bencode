//! Standalone transcript rows: the user's message, the agent's answer,
//! notices, and the footer a finished turn leaves (MonoCode
//! `TranscriptBlock`, `UserMessage`, `TurnDuration`).

use ely_gpui_component::chat::{CodeBlock, StreamingMarkdown};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};
use jiff::Timestamp;

use super::turns::{self, TurnLayout};
use crate::app::BenCodeApp;
use crate::db::SessionRow;

/// MonoCode's transcript column (`max-w-4xl`).
pub const MESSAGE_MAX_WIDTH: gpui::Pixels = px(896.0);
/// MonoCode's user bubble width (`min(100%, 36rem)`).
const USER_BUBBLE_MAX_WIDTH: gpui::Pixels = px(576.0);
/// Long messages clamp to this many lines until "Show more".
const CLAMP_LINES: usize = 4;
/// How long a copy button shows its check (MonoCode).
const COPIED_FOR: std::time::Duration = std::time::Duration::from_secs(2);
/// Rough characters per bubble line, to tell when a message needs the clamp.
const CHARS_PER_LINE: usize = 72;

pub fn markdown(id: SharedString, text: &str, live: bool) -> AnyElement {
    if live {
        return StreamingMarkdown::new(id, text.to_string(), true).into_any_element();
    }
    MarkdownRenderer::new(id, text.to_string())
        .code(|id, language, code| {
            let block = CodeBlock::new(id, code);
            match language {
                Some(language) => block.language(language),
                None => block,
            }
            .into_any_element()
        })
        .into_any_element()
}

fn muted(color: Hsla, opacity: f32) -> Hsla {
    color.opacity(opacity)
}

/// Local `H:MM` of an epoch-ms time.
fn clock_time(at_ms: i64) -> Option<String> {
    let ts = Timestamp::from_millisecond(at_ms).ok()?;
    let zdt = ts.to_zoned(jiff::tz::TimeZone::system());
    Some(format!("{}:{:02}", zdt.hour(), zdt.minute()))
}

/// Lines a message wraps to, roughly, to decide on the 4-line clamp.
fn approx_lines(text: &str) -> usize {
    text.lines()
        .map(|line| line.chars().count().div_ceil(CHARS_PER_LINE).max(1))
        .sum()
}

/// A 22px icon button for transcript actions.
fn action_button(
    id: String,
    icon: IconName,
    tooltip: &'static str,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&mut BenCodeApp, &mut Context<BenCodeApp>) + 'static,
) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let hover_bg = muted(colors.fg, 0.08);
    div()
        .id(SharedString::from(id))
        .size(px(22.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .tooltip(Tooltip::text(tooltip))
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(
            Icon::new(icon)
                .size(IconSize::Xs)
                .color(muted(colors.fg, 0.4)),
        )
}

/// MonoCode `CopyTurnButton`: copies, then shows a check for two seconds.
fn copy_button(
    id: String,
    text: String,
    copied: bool,
    cx: &Context<BenCodeApp>,
) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let hover_bg = muted(colors.fg, 0.08);
    let key = id.clone();
    div()
        .id(SharedString::from(id))
        .size(px(22.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .tooltip(Tooltip::text(if copied { "Copied" } else { "Copy" }))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
            this.transcript_ui.copied.insert(key.clone());
            let key = key.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(COPIED_FOR).await;
                let reset = this.update(cx, |this, cx| {
                    this.transcript_ui.copied.remove(&key);
                    cx.notify();
                });
                if let Err(err) = reset {
                    log::debug!("copy reset after app drop: {err:#}");
                }
            })
            .detach();
            cx.notify();
        }))
        .child(if copied {
            Icon::new(IconName::Check)
                .size(IconSize::Xs)
                .color(colors.success)
        } else {
            Icon::new(IconName::Copy)
                .size(IconSize::Xs)
                .color(muted(colors.fg, 0.4))
        })
}

fn dot(cx: &App) -> impl IntoElement {
    div()
        .size(px(3.0))
        .flex_none()
        .rounded_full()
        .bg(muted(cx.theme().colors.fg, 0.25))
}

impl BenCodeApp {
    /// Block `ix` on its own row. `under_work` puts prose right under the
    /// work it follows (MonoCode `underLine`).
    pub(super) fn render_block(
        &self,
        session: &SessionRow,
        ix: usize,
        live: bool,
        under_work: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &session.blocks[ix];
        match block.role.as_str() {
            "user" => self.render_user_message(session, ix, cx),
            "system" => notice(turns::text(block), cx),
            _ => prose(session, ix, live, under_work, cx),
        }
    }

    /// MonoCode's user bubble: right-aligned, clamped to four lines with
    /// Show more, and a row of actions that fades in on hover.
    fn render_user_message(
        &self,
        session: &SessionRow,
        ix: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &session.blocks[ix];
        let text = turns::text(block).to_string();
        let colors = &cx.theme().colors;
        let key = block.id.clone();
        let clamps = approx_lines(&text) > CLAMP_LINES;
        let expanded = self.transcript_ui.expanded_messages.contains(&key);
        let single_line = !text.contains('\n') && text.chars().count() <= CHARS_PER_LINE;
        let group = SharedString::from(format!("user-msg-{key}"));
        let bubble = div()
            .min_w_0()
            .max_w(USER_BUBBLE_MAX_WIDTH)
            .px_3()
            .py_2()
            .bg(muted(colors.fg, 0.1))
            .rounded(if single_line { px(18.0) } else { px(12.0) })
            .text_size(px(14.0))
            .line_height(px(22.0))
            .text_color(colors.fg)
            .child(
                div()
                    .when(clamps && !expanded, |el| el.line_clamp(CLAMP_LINES))
                    .child(text.clone()),
            )
            .when(clamps, |el| {
                el.child(self.show_more_toggle(&key, expanded, cx))
            });
        let draft = block.extra.get("draft").and_then(|d| d.as_bool()) == Some(true);
        let bubble = if draft {
            bubble
                .border_1()
                .border_dashed()
                .border_color(colors.fg.opacity(0.3))
        } else {
            bubble
        };
        div()
            .id(SharedString::from(format!("user-row-{key}")))
            .group(group.clone())
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .pt_1p5()
            .pr_4()
            .pb_1()
            .pl(px(56.0))
            .children(attachment_chips(block, cx))
            .child(bubble)
            .child(if draft {
                self.draft_actions(session, ix, cx).into_any_element()
            } else {
                self.user_actions(session, ix, text, &group, cx)
                    .into_any_element()
            })
            .into_any_element()
    }

    /// A saved draft: "Draft", then Send or Remove (MonoCode drafts).
    fn draft_actions(
        &self,
        session: &SessionRow,
        ix: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let block_id = session.blocks[ix].id.clone();
        let (send_sid, send_bid) = (session.id.clone(), block_id.clone());
        let (drop_sid, drop_bid) = (session.id.clone(), block_id.clone());
        let button = |id: String, label: &'static str, primary: bool| {
            let (bg, fg) = if primary {
                (colors.fg, colors.bg)
            } else {
                (colors.fg.opacity(0.1), colors.fg.opacity(0.7))
            };
            div()
                .id(SharedString::from(id))
                .px_2()
                .h(px(22.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .bg(bg)
                .text_color(fg)
                .text_size(px(11.0))
                .cursor_pointer()
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.0))
                    .text_color(colors.fg.opacity(0.55))
                    .child(
                        Icon::new(IconName::CircleDashed)
                            .size(IconSize::Xs)
                            .color(colors.fg.opacity(0.55)),
                    )
                    .child("Draft"),
            )
            .child(
                button(format!("draft-send-{block_id}"), "Send", true).on_click(
                    cx.listener(move |this, _, _, cx| this.send_draft(&send_sid, &send_bid, cx)),
                ),
            )
            .child(
                button(format!("draft-remove-{block_id}"), "Remove", false).on_click(
                    cx.listener(move |this, _, _, cx| this.remove_draft(&drop_sid, &drop_bid, cx)),
                ),
            )
    }

    fn show_more_toggle(&self, key: &str, expanded: bool, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let key = key.to_string();
        let hover = muted(colors.fg, 0.08);
        div()
            .id(SharedString::from(format!("show-more-{key}")))
            .mt_1()
            .px_1()
            .rounded(px(4.0))
            .text_size(px(12.0))
            .text_color(muted(colors.fg, 0.6))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                let set = &mut this.transcript_ui.expanded_messages;
                if !set.remove(&key) {
                    set.insert(key.clone());
                }
                cx.notify();
            }))
            .child(if expanded { "Show less" } else { "Show more" })
    }

    /// Copy, edit (the last message, once settled), save note, time: hidden
    /// until the message is hovered.
    fn user_actions(
        &self,
        session: &SessionRow,
        ix: usize,
        text: String,
        group: &SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let block = &session.blocks[ix];
        let last_user = session.blocks.iter().rposition(|b| b.role == "user") == Some(ix);
        let editable = last_user && !self.is_agent_running_in(&session.id);
        let (edit_text, note_text) = (text.clone(), text.clone());
        let time = block.started_at.and_then(clock_time);
        let colors = &cx.theme().colors;
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .h(px(24.0))
            .opacity(0.0)
            .group_hover(group.clone(), |s| s.opacity(1.0))
            .child({
                let id = format!("{}-copy", block.id);
                let copied = self.transcript_ui.copied.contains(&id);
                copy_button(id, text, copied, cx)
            })
            .when(editable, |el| {
                el.child(action_button(
                    format!("{}-edit", block.id),
                    IconName::Pencil,
                    "Edit and resend",
                    cx,
                    move |this, cx| this.edit_turn(&edit_text, cx),
                ))
            })
            .child(action_button(
                format!("{}-note", block.id),
                IconName::FilePlus,
                "Save as note",
                cx,
                move |this, cx| this.save_turn_to_note(&note_text, cx),
            ))
            .when_some(time, |el, time| {
                el.child(
                    div()
                        .pl_1()
                        .text_size(px(12.0))
                        .text_color(muted(colors.fg, 0.4))
                        .child(time),
                )
            })
    }

    /// MonoCode `TurnDuration`: copy and save the turn's text, then the time
    /// it finished. The clock itself lives on the fold line above.
    pub(super) fn render_turn_footer(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        cx: &Context<Self>,
    ) -> AnyElement {
        let blocks = &session.blocks;
        let copy = turns::turn_copy_text(blocks, turn.range.clone());
        let finished = turn
            .started_at(blocks)
            .zip(turn.duration_ms(blocks))
            .and_then(|(start, ms)| clock_time(start + ms));
        let id = turn.id(blocks).to_string();
        let colors = &cx.theme().colors;
        let note_text = copy.clone();
        div()
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2p5()
            .px_4()
            .pt_1()
            .pb_3()
            .text_size(px(14.0))
            .text_color(muted(colors.fg, 0.4))
            .when(!copy.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child({
                            let key = format!("turn-copy-{id}");
                            let copied = self.transcript_ui.copied.contains(&key);
                            copy_button(key, copy, copied, cx)
                        })
                        .child(action_button(
                            format!("turn-note-{id}"),
                            IconName::FilePlus,
                            "Save as note",
                            cx,
                            move |this, cx| this.save_turn_to_note(&note_text, cx),
                        )),
                )
            })
            .when_some(finished, |el, time| {
                el.child(dot(cx)).child(
                    div()
                        .flex_none()
                        .text_color(muted(colors.fg, 0.35))
                        .child(time),
                )
            })
            .into_any_element()
    }
}

/// The files a message was sent with, as small chips above the bubble.
fn attachment_chips(block: &crate::db::Block, cx: &Context<BenCodeApp>) -> Option<AnyElement> {
    let files = block.extra.get("attachments")?.as_array()?;
    if files.is_empty() {
        return None;
    }
    let colors = &cx.theme().colors;
    let chips = files.iter().filter_map(|file| {
        let name = file.get("name")?.as_str()?.to_string();
        let image = file.get("kind").and_then(|k| k.as_str()) == Some("image");
        Some(
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(22.0))
                .px_1p5()
                .max_w(px(220.0))
                .rounded(px(6.0))
                .bg(colors.fg.opacity(0.08))
                .text_size(px(12.0))
                .text_color(colors.fg.opacity(0.75))
                .child(
                    Icon::new(if image {
                        IconName::Image
                    } else {
                        IconName::FileText
                    })
                    .size(IconSize::Xs)
                    .color(colors.fg_muted),
                )
                .child(div().min_w_0().truncate().child(name)),
        )
    });
    Some(
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap_1p5()
            .children(chips)
            .into_any_element(),
    )
}

/// The agent's answer at full strength: 14px on 24px lines.
fn prose(
    session: &SessionRow,
    ix: usize,
    live: bool,
    under_work: bool,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let block = &session.blocks[ix];
    let id = SharedString::from(format!("{}-{ix}", session.id));
    div()
        .px_4()
        .when(under_work, |el| el.pt_1())
        .when(!under_work, |el| el.pt_3())
        .text_size(px(14.0))
        .line_height(px(24.0))
        .text_color(cx.theme().colors.fg)
        .child(markdown(id, turns::text(block), live))
        .into_any_element()
}

/// A notice the reader must not miss, as plain muted text (MonoCode).
fn notice(text: &str, cx: &Context<BenCodeApp>) -> AnyElement {
    div()
        .px_4()
        .py_2()
        .text_size(px(14.0))
        .text_color(muted(cx.theme().colors.fg, 0.5))
        .child(text.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_need_the_clamp() {
        assert!(approx_lines("short") <= CLAMP_LINES);
        assert!(approx_lines("a\nb\nc\nd\ne") > CLAMP_LINES);
        assert!(approx_lines(&"x".repeat(CHARS_PER_LINE * 5)) > CLAMP_LINES);
    }
}
