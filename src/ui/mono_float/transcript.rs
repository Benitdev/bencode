//! The floating chat's rows as MonoCode draws its Mono transcript
//! (`AgentTranscript` with `agentMascot`, `inlineWork` and `bubbleTail`,
//! and the `mono-*` classes of `index.css`): the user's message a bubble at
//! the right, its photos above it, Copy and its time under it on hover; the
//! Mono's turn signed by its pill (`MonoSignaturePill`), with the rolling
//! ticker beside it while it works (`MonoTurnHeader`, `MonoWorkTicker`);
//! its reply; and under a settled turn, on hover, Copy, Show activity and
//! the time it finished (`TurnDuration` `revealOnHover`).
//!
//! GPUI has no masks, so the transcript's fades at its top and bottom
//! (`mono-transcript-fades`) are the card's colour drawn over it, and the
//! reaction badge's frosted glass is a solid shade; bubbles and reactions
//! come in without their pop.

use std::path::PathBuf;
use std::time::Instant;

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Hsla, InteractiveElement, IntoElement,
    ObjectFit, ParentElement, Pixels, SharedString, Stateful, StatefulInteractiveElement, Styled,
    StyledImage, TextRun, Window, div, linear_color_stop, linear_gradient, list, prelude::*,
    relative,
};

use super::items::{Delivery, FloatItem, FloatRow};
use super::ticker::TICK;
use super::{CARD_GAP, MonoFloat, RAIL_WIDTH};
use crate::monos::is_emoji_only;
use crate::ui::attachment_chip::{ChipFile, attachment_chip};
use crate::ui::mascot::pixel_sprite;
use crate::ui::scale::px;
use crate::ui::thumbnail::thumbnail;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::ui::transcript::mono::{
    clock_time, day_separator, reaction_badge, signature_name, signature_pill,
};

/// MonoCode `user-message-row`'s `pl-14 pr-4`.
const MESSAGE_LEFT: f32 = 56.0;
const MESSAGE_RIGHT: f32 = 16.0;
/// `max-w-[min(100%,36rem)]`.
const BUBBLE_MAX: f32 = 576.0;
/// The bubble's `px-3`.
const BUBBLE_PAD: f32 = 12.0;
/// `text-sm`, and the `line-clamp-4` a long message folds to.
const MESSAGE_SIZE: f32 = 14.0;
const MESSAGE_LINE: f32 = 20.0;
const CLAMP: usize = 4;
/// A photo's largest side (`max-h-60 max-w-60`), and its box while its
/// size is read.
const PHOTO: f32 = 240.0;
const PHOTO_LOADING: (f32, f32) = (PHOTO, 180.0);
/// `.mono-work-ticker`: one 24px line.
const TICKER_LINE: f32 = 24.0;
/// `mono-transcript-fades`: `1.5rem` at the top, `2rem` at the bottom.
const FADE_TOP: f32 = 24.0;
const FADE_BOTTOM: f32 = 32.0;
/// The scroller's `pb-5`: the last row stops above the bottom fade.
const LIST_END: f32 = 20.0;

/// Which message a "Show more" opened: by its turn, and by its row while it
/// waits to be sent (it has no turn yet).
pub(super) fn bubble_key(turn: usize, ix: usize) -> (usize, usize) {
    if turn == usize::MAX {
        (turn, ix)
    } else {
        (turn, 0)
    }
}

/// How many lines `text` takes at `width` in the message's type.
fn line_count(text: &str, width: Pixels, window: &Window) -> usize {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    match window.text_system().shape_text(
        SharedString::from(text.to_string()),
        px(MESSAGE_SIZE),
        &[run],
        Some(width),
        None,
    ) {
        Ok(lines) => lines
            .iter()
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum(),
        Err(err) => {
            log::debug!("could not measure a message: {err:#}");
            1
        }
    }
}

/// `photo` fitted inside the photo box, keeping its shape.
fn photo_size(natural: Option<(f32, f32)>) -> (f32, f32) {
    let Some((width, height)) = natural.filter(|(w, h)| *w > 0.0 && *h > 0.0) else {
        return PHOTO_LOADING;
    };
    let scale = (PHOTO / width).min(PHOTO / height).min(1.0);
    (width * scale, height * scale)
}

/// MonoCode `CopyTurnButton`: `rounded-md p-1 text-content/40
/// hover:bg-content/8 hover:text-content/70`, a check once copied.
fn quiet_button(
    id: SharedString,
    icon: IconName,
    tip: &'static str,
    cx: &Context<MonoFloat>,
) -> Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    div()
        .id(id.clone())
        .group(id.clone())
        .flex()
        .flex_none()
        .p_1()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(fg.fill(Fill::Selected)))
        .tooltip(Tooltip::text(tip))
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Muted))
                .group_hover_color(id, fg.ink(Ink::Body)),
        )
}

/// `size-[3px] rounded-full bg-content/25`: what parts a turn's footer.
fn footer_dot(fg: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(3.0))
        .rounded_full()
        .bg(fg.ink(Ink::Faint))
}

impl MonoFloat {
    /// The transcript under the header: the rows with their fades, the
    /// question form or the usage-limit notice, then the composer.
    pub(super) fn render_conversation(
        &mut self,
        mono: &super::MonoFace,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bg = cx.theme().colors.bg;
        let body = if self.rows.is_empty() {
            self.render_placeholder(
                Some(mono),
                format!(
                    "Say hello to {}, ask a question, or hand over some work.",
                    mono.name
                ),
                cx,
            )
        } else {
            let fade = |angle: f32| {
                linear_gradient(
                    angle,
                    linear_color_stop(bg, 0.0),
                    linear_color_stop(bg.opacity(0.0), 1.0),
                )
            };
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, ix: usize, window, cx| {
                            this.render_item(ix, window, cx)
                        }),
                    )
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(FADE_TOP))
                        .bg(fade(180.0)),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(FADE_BOTTOM))
                        .bg(fade(0.0)),
                )
                .into_any_element()
        };
        div()
            .id("mono-float-conversation")
            .track_focus(&self.conversation)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(body)
            .children(self.render_question(session_id, cx))
            .children(self.render_usage_limit(session_id, cx))
            .child(self.render_composer(session_id, window, cx))
            .into_any_element()
    }

    pub(super) fn render_item(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(FloatRow { turn, item }) = self.rows.get(ix).cloned() else {
            // The list's end, under the last row.
            return div().h(px(LIST_END)).into_any_element();
        };
        let fg = cx.theme().colors.fg;
        let row = div().w_full().px_4();
        let body = match item {
            FloatItem::Earlier => div()
                .flex()
                .w_full()
                .justify_center()
                .px_4()
                .py_3()
                .child(
                    Button::new("mono-float-earlier", "Load earlier messages")
                        .variant(ButtonVariant::Secondary)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.load_earlier(cx))),
                )
                .into_any_element(),
            FloatItem::User {
                text,
                files,
                reaction,
                delivery,
                at,
            } => {
                return self.render_message(
                    ix,
                    turn,
                    Message {
                        text: &text,
                        files: &files,
                        reaction: reaction.as_deref(),
                        delivery: &delivery,
                        at,
                    },
                    window,
                    cx,
                );
            }
            FloatItem::Day(at) => {
                // Its turn's actions stay hidden over it.
                return day_separator(("mono-float-day", ix), at, cx)
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hover_message(ix, *hovered, cx);
                    }))
                    .into_any_element();
            }
            FloatItem::Header { ticker } => row
                .flex()
                .items_center()
                .gap_1p5()
                .py_1()
                .text_size(Text::Lg.px())
                .text_color(fg.ink(Ink::Secondary))
                .map(|el| match ticker {
                    // MonoCode `MonoTurnHeader` while active: the pill holds
                    // the Mono's signature, the ticker runs beside it.
                    Some(label) => el
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .min_w_0()
                                .max_w(relative(0.45))
                                .child(self.render_sign(cx)),
                        )
                        .child(div().flex_none().text_color(fg.ink(Ink::Faint)).child("·"))
                        .child(
                            // MonoCode `onShowWork`: the trail is in BenCode.
                            div()
                                .id(("mono-float-ticker", ix))
                                .min_w_0()
                                .flex_1()
                                .cursor_pointer()
                                .hover(|s| s.opacity(0.7))
                                .tooltip(Tooltip::text("Show activity"))
                                .on_click(cx.listener(|this, _, _, cx| this.reveal(cx)))
                                .child(self.render_ticker(&label, cx)),
                        ),
                    None => el.child(self.render_sign(cx)),
                })
                .into_any_element(),
            FloatItem::Reply(text) => row
                .pt_1()
                .child(div().text_size(Text::Md.px()).child(
                    AgentMarkdown::new(("mono-float-reply", ix), text).on_open_file(
                        std::rc::Rc::new({
                            let this = cx.entity().downgrade();
                            move |path, _line, _, cx| {
                                let opened = this.update(cx, |this, cx| this.reveal_file(path, cx));
                                if let Err(err) = opened {
                                    log::debug!("float file link after close: {err:#}");
                                }
                            }
                        }),
                    ),
                ))
                .into_any_element(),
            FloatItem::Notice(text) => row
                .pt_2()
                .text_size(Text::Base.px())
                .line_height(px(18.0))
                .text_color(fg.ink(Ink::Muted))
                .child(text)
                .into_any_element(),
            FloatItem::Approval(_) => {
                let session_id = self.shown.session.clone().unwrap_or_default();
                let prompt = self
                    .app
                    .update(cx, |app, cx| app.render_permission_prompt(&session_id, cx));
                div().w_full().pt_1().children(prompt).into_any_element()
            }
            FloatItem::HabitApproval { run, habit } => {
                let prompt = self
                    .app
                    .update(cx, |app, cx| app.render_habit_approval(&run, &habit, cx));
                div().w_full().pt_1().children(prompt).into_any_element()
            }
            // MonoCode's sheet opens in the chat; here the card opens it
            // over the Mono's Overview in BenCode's window.
            FloatItem::Artifact(card) => {
                let id = card.id.clone();
                row.pt_2()
                    .flex()
                    .child(
                        crate::ui::artifacts::artifact_card(
                            format!("mono-float-artifact-{ix}"),
                            &card,
                            cx,
                        )
                        .tooltip(Tooltip::text("Open in BenCode"))
                        .on_click(cx.listener(move |this, _, _, cx| this.reveal_artifact(&id, cx))),
                    )
                    .into_any_element()
            }
            FloatItem::Launch(launch) => {
                let id = launch.session_id.clone();
                let app = self.app.read(cx);
                let (state, title) = (app.launched_state(&id), app.launched_title(&launch));
                row.pt_2()
                    .flex()
                    .child(
                        crate::ui::launched_session::launched_card(
                            format!("mono-float-launch-{ix}"),
                            &launch,
                            title,
                            state,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.reveal_launch(&id, cx))),
                    )
                    .into_any_element()
            }
            FloatItem::Footer { copy, at } => self.render_footer(ix, turn, copy, at, cx),
        };
        if turn == usize::MAX {
            return body;
        }
        // Hovering the turn's work, reply or footer shows its actions.
        div()
            .id(("mono-float-row", ix))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let next = if *hovered {
                    Some(turn)
                } else if this.hovered_turn == Some(turn) {
                    None
                } else {
                    return;
                };
                if this.hovered_turn != next {
                    this.hovered_turn = next;
                    cx.notify();
                }
            }))
            .child(body)
            .into_any_element()
    }

    /// The pointer on a message or a day stamp: their turn's actions hide
    /// (`:has([data-prompt-anchor]:hover, [data-day-separator]:hover)`).
    fn hover_message(&mut self, ix: usize, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(ix)
        } else if self.hovered_message == Some(ix) {
            None
        } else {
            return;
        };
        if self.hovered_message != next || (hovered && self.hovered_turn.is_some()) {
            self.hovered_message = next;
            if hovered {
                self.hovered_turn = None;
            }
            cx.notify();
        }
    }

    /// MonoCode `MonoWorkTicker`: one clipped 24px line, the new status
    /// rolling the one before it up and out.
    fn render_ticker(&self, label: &str, cx: &Context<Self>) -> AnyElement {
        let line = |text: &str| {
            div()
                .h(px(TICKER_LINE))
                .flex()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(crate::ui::spinner::shimmer(text.to_string(), cx)),
                )
        };
        let frame = self.ticker.frame(Instant::now());
        let current = frame.as_ref().map_or(label, |frame| frame.current);
        let clip = div().h(px(TICKER_LINE)).overflow_hidden();
        match frame
            .as_ref()
            .and_then(|frame| Some((frame.previous?, frame.sequence)))
        {
            Some((previous, sequence)) => clip
                .child(
                    div()
                        .relative()
                        .child(line(previous))
                        .child(line(current))
                        .with_animation(
                            ("mono-float-tick", sequence),
                            Animation::new(TICK).with_easing(gpui::ease_out_quint()),
                            |el, delta| el.top(px(-TICKER_LINE * delta)),
                        ),
                )
                .into_any_element(),
            None => clip.child(line(current)).into_any_element(),
        }
    }

    /// MonoCode `MonoSignaturePill`: `rounded-full bg-content/[0.07] py-0.5
    /// pr-3 pl-2`, the still mascot at `size-3.5` and the name in
    /// `font-medium text-content/80`.
    fn render_sign(&self, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let Some(mono) = app.mono(&self.mono_id) else {
            return div().into_any_element();
        };
        let mark = pixel_sprite(
            &app.mono_mascot(mono).rest,
            px(14.0),
            app.mono_color(mono),
            false,
        );
        signature_pill(mark, signature_name(mono.display_name(), cx), cx).into_any_element()
    }

    /// MonoCode `TurnDuration` with `revealOnHover`: Copy (a check when
    /// the turn said nothing to copy), Show activity, then the time it
    /// finished at 35%, parted by 3px dots.
    fn render_footer(
        &self,
        ix: usize,
        turn: usize,
        copy: String,
        at: Option<i64>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let shown = self.hovered_turn == Some(turn);
        let key = SharedString::from(format!("mono-float-copy-{ix}"));
        let copied = self.is_copied(&key);
        div()
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(px(10.0))
            .h(px(24.0))
            .px_4()
            .text_size(Text::Lg.px())
            .text_color(fg.ink(Ink::Muted))
            .when(!shown, |el| el.invisible())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .map(|el| {
                        if copy.is_empty() {
                            el.child(
                                Icon::new(IconName::Check)
                                    .size(IconSize::Sm)
                                    .color(fg.ink(Ink::Muted)),
                            )
                        } else {
                            let tip = if copied { "Copied" } else { "Copy response" };
                            let icon = if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            };
                            el.child(quiet_button(key.clone(), icon, tip, cx).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.copy(key.clone(), copy.clone(), cx);
                                }),
                            ))
                        }
                    })
                    .child(
                        quiet_button(
                            SharedString::from(format!("mono-float-activity-{ix}")),
                            IconName::List,
                            "Show activity",
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.reveal(cx))),
                    ),
            )
            .children(at.and_then(clock_time).map(|time| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .child(footer_dot(fg))
                    .child(div().text_color(fg.ink(Ink::Faint)).child(time))
            }))
            .into_any_element()
    }

    /// MonoCode `UserMessageBlock` in a Mono's chat.
    fn render_message(
        &mut self,
        ix: usize,
        turn: usize,
        message: Message<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Message {
            text,
            files,
            reaction,
            delivery,
            at,
        } = message;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let dark = cx.theme().is_dark();
        // A message followed straight by another stacks on it, without
        // actions (`compactFollowUp`).
        let compact = matches!(
            self.rows.get(ix + 1),
            Some(FloatRow {
                item: FloatItem::User { .. },
                ..
            })
        );
        let hovered = self.hovered_message == Some(ix);
        let row = div()
            .id(("mono-float-message", ix))
            .flex()
            .flex_col()
            .items_end()
            .w_full()
            .pl(px(MESSAGE_LEFT))
            .pr(px(MESSAGE_RIGHT))
            .map(|el| {
                if compact {
                    el.pt_1()
                } else {
                    el.pt_1p5().pb_5()
                }
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hover_message(ix, *hovered, cx);
            }));
        let control = self.delivery_control(ix, delivery, cx);
        // A few emoji stand alone, like a reaction: no bubble, time or
        // actions.
        if files.is_empty() && is_emoji_only(text) {
            return row
                .child(
                    div()
                        .relative()
                        .text_size(px(60.0))
                        .line_height(px(60.0))
                        .child(text.to_string())
                        .children(reaction.map(|emoji| reaction_badge(emoji, cx))),
                )
                .children(control)
                .into_any_element();
        }
        // Sent images show above the bubble, like photos; other files stay
        // inside it. Images sent on their own need no bubble.
        let (photos, others): (Vec<&ChipFile>, Vec<&ChipFile>) =
            files.iter().partition(|file| file.image);
        let bubble_empty = !photos.is_empty() && text.is_empty() && others.is_empty();
        // The bubble's width, as GPUI will lay it out.
        let card = window.bounds().size.width - px(RAIL_WIDTH + CARD_GAP);
        let bubble_max = (card - px(MESSAGE_LEFT + MESSAGE_RIGHT)).min(px(BUBBLE_MAX));
        let lines = if text.is_empty() {
            0
        } else {
            line_count(text, bubble_max - px(BUBBLE_PAD * 2.0), window)
        };
        let key = bubble_key(turn, ix);
        let expanded = self.expanded.contains(&key);
        let overflows = lines > CLAMP;
        // MonoCode rounds a bubble of one line of text fully.
        let single = lines == 1 && others.is_empty();
        let accent = crate::ui::appearance::user_accent(cx);
        // `.agent-chat-user-bubble`: content at 10% (black in light), or
        // the accent the user picked at 24%.
        let (bubble_bg, bubble_fg) = match accent {
            Some(accent) => (accent.color.opacity(0.24), fg),
            None if dark => (fg.fill(Fill::Strong), fg),
            None => (gpui::rgb(0x0a0a0a).into(), gpui::white()),
        };
        let others: Vec<ChipFile> = others.into_iter().cloned().collect();
        let chips = self.chips(&others, None, cx);
        let photos: Vec<AnyElement> = photos
            .into_iter()
            .enumerate()
            .map(|(at, file)| self.render_photo(ix, at, file, cx))
            .collect();
        let copy_key = SharedString::from(format!("mono-float-copy-message-{ix}"));
        let copied = self.is_copied(&copy_key);
        let copy_text = text.to_string();
        let actions =
            (hovered && !compact && (!text.is_empty() || !files.is_empty() || at.is_some())).then(
                || {
                    // `.user-message-actions`: under the message, in the row's
                    // gap, so showing them moves nothing.
                    div()
                        .absolute()
                        .top_full()
                        .right_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .pt_1()
                        .when(!text.is_empty(), |el| {
                            let tip = if copied { "Copied" } else { "Copy message" };
                            let icon = if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            };
                            el.child(quiet_button(copy_key.clone(), icon, tip, cx).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.copy(copy_key.clone(), copy_text.clone(), cx);
                                }),
                            ))
                        })
                        .children(at.and_then(clock_time).map(|time| {
                            div()
                                .ml_1()
                                .text_size(Text::Base.px())
                                .text_color(fg.ink(Ink::Muted))
                                .child(time)
                        }))
                },
            );
        let bubble = (!bubble_empty).then(|| {
            div()
                .relative()
                .min_w_0()
                .max_w(bubble_max)
                .px(px(BUBBLE_PAD))
                .py_2()
                .bg(bubble_bg)
                .text_color(bubble_fg)
                .rounded(if single { px(999.0) } else { px(12.0) })
                .when(!chips.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1p5()
                            .when(!text.is_empty(), |el| el.mb_2())
                            .children(chips),
                    )
                })
                .when(!text.is_empty(), |el| {
                    el.child(
                        div()
                            .min_w_0()
                            .text_size(px(MESSAGE_SIZE))
                            .line_height(px(MESSAGE_LINE))
                            .when(overflows && !expanded, |el| el.line_clamp(CLAMP))
                            .child(text.to_string()),
                    )
                })
                .when(overflows, |el| {
                    el.child(
                        div()
                            .id(("mono-float-more", ix))
                            .mt_1()
                            .px_1()
                            .py_0p5()
                            .rounded(px(4.0))
                            .text_size(Text::Base.px())
                            .text_color(bubble_fg.opacity(0.6))
                            .cursor_pointer()
                            .hover(move |s| s.bg(bubble_fg.opacity(0.08)).text_color(bubble_fg))
                            .child(if expanded { "Show less" } else { "Show more" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&key) {
                                    this.expanded.insert(key);
                                }
                                this.list.remeasure_items(ix..ix + 1);
                                cx.notify();
                            })),
                    )
                })
                .children(reaction.map(|emoji| reaction_badge(emoji, cx)))
        });
        row.child(
            // `.user-message-hover-zone`: the actions hang under it.
            div()
                .relative()
                .flex()
                .flex_col()
                .items_end()
                .max_w_full()
                .when(!photos.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_1p5()
                            .when(bubble.is_some(), |el| el.mb_1())
                            .children(photos),
                    )
                })
                .children(bubble)
                .children(control)
                .children(actions),
        )
        .into_any_element()
    }

    /// MonoCode's delivery control under a message that was not sent:
    /// "Not sent" while held since the stop, "Not sent · why" with Retry
    /// when it could not be sent. A message waiting its turn has none.
    fn delivery_control(
        &self,
        ix: usize,
        delivery: &Delivery,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let fg = cx.theme().colors.fg;
        let line = match delivery {
            Delivery::Sent | Delivery::Pending => return None,
            Delivery::Paused => "Not sent".to_string(),
            Delivery::Failed(error) => format!("Not sent · {error}"),
        };
        let failed = matches!(delivery, Delivery::Failed(_));
        Some(
            div()
                .flex()
                .max_w_full()
                .items_center()
                .gap_2()
                .px_3()
                .pt_1()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Secondary))
                .child(div().min_w_0().child(line))
                .when(failed, |el| {
                    el.child(
                        div()
                            .id(("mono-float-retry-send", ix))
                            .flex_none()
                            .px_1()
                            .py_0p5()
                            .rounded(px(4.0))
                            .text_color(fg.ink(Ink::Body))
                            .cursor_pointer()
                            .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                            .child("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                    )
                })
                .into_any_element(),
        )
    }

    /// MonoCode `AttachmentChip` with `photo`: a sent image large, at its
    /// own shape, `rounded-2xl`; a click opens it in BenCode's preview.
    fn render_photo(
        &self,
        ix: usize,
        at: usize,
        file: &ChipFile,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (width, height) = photo_size(self.photo_sizes.get(&file.path).copied().flatten());
        let path = PathBuf::from(&file.path);
        div()
            .id(SharedString::from(format!("mono-float-photo-{ix}-{at}")))
            .flex_none()
            .w(px(width))
            .h(px(height))
            .rounded(px(16.0))
            .overflow_hidden()
            .bg(cx.theme().colors.fg.fill(Fill::Hover))
            .cursor_pointer()
            .tooltip(Tooltip::text(file.name.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_main(cx);
                let path = path.clone();
                this.app.update(cx, |app, cx| app.open_lightbox(path, cx));
            }))
            .child(
                thumbnail(&file.path, PHOTO)
                    .size_full()
                    .rounded(px(16.0))
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element()
    }

    /// Reads the size of each photo shown that has none yet, off the UI
    /// thread, and lays its row out again once it lands.
    pub(super) fn read_photo_sizes(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| match &row.item {
                FloatItem::User { files, .. } => Some(files),
                _ => None,
            })
            .flatten()
            .filter(|file| file.image && !self.photo_sizes.contains_key(&file.path))
            .map(|file| file.path.clone())
            .collect();
        if missing.is_empty() {
            return;
        }
        for path in &missing {
            self.photo_sizes.insert(path.clone(), None);
        }
        let task = cx.background_executor().spawn(async move {
            missing
                .into_iter()
                .map(|path| {
                    let size = image::image_dimensions(&path)
                        .map_err(|err| log::debug!("no size for {path}: {err}"))
                        .ok()
                        .map(|(w, h)| (w as f32, h as f32));
                    (path, size)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let sizes = task.await;
            let landed = this.update(cx, |this, cx| {
                for (path, size) in sizes {
                    // A size that could not be read keeps the loading box.
                    this.photo_sizes.insert(path, size.or(Some(PHOTO_LOADING)));
                }
                this.list.remeasure();
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("photo sizes after the floating chat closed: {err:#}");
            }
        })
        .detach();
    }

    /// Files as the composer's chips: removable while they wait in
    /// `session_id`'s composer, plain on a sent message.
    pub(super) fn chips(
        &self,
        files: &[ChipFile],
        session_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // The chip is the app's own widget and binds its clicks to the app.
        self.app.update(cx, |_, cx| {
            files
                .iter()
                .map(|file| {
                    let remove = session_id.map(|session_id| {
                        let (sid, id) = (session_id.to_string(), file.id.clone());
                        let remove: crate::ui::attachment_chip::OnRemove =
                            std::rc::Rc::new(move |app, cx| app.remove_attachment(&sid, &id, cx));
                        remove
                    });
                    attachment_chip(file, remove, cx)
                })
                .collect()
        })
    }
}

/// What a message row draws.
struct Message<'a> {
    text: &'a str,
    files: &'a [ChipFile],
    reaction: Option<&'a str>,
    delivery: &'a Delivery,
    at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photo_keeps_its_shape_inside_the_box() {
        assert_eq!(photo_size(Some((480.0, 240.0))), (240.0, 120.0));
        assert_eq!(photo_size(Some((120.0, 480.0))), (60.0, 240.0));
        // Never drawn larger than it is.
        assert_eq!(photo_size(Some((80.0, 40.0))), (80.0, 40.0));
        assert_eq!(photo_size(None), PHOTO_LOADING);
        assert_eq!(photo_size(Some((0.0, 10.0))), PHOTO_LOADING);
    }

    #[test]
    fn a_waiting_message_is_opened_by_its_row() {
        assert_eq!(bubble_key(4, 9), (4, 0));
        assert_eq!(bubble_key(usize::MAX, 9), (usize::MAX, 9));
    }
}
