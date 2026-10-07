//! MonoCode `PromptOutline`: a stack of short bars at the transcript's
//! right edge, one per prompt. The bar of the prompt in view is lit; the
//! hovered one lifts with its neighbours (a dock-style ripple) and opens a
//! card with the prompt and the head of its reply; a click scrolls to that
//! turn. The rail is one tab stop: ↑/↓ walk it, Enter jumps. The model is
//! `outline_model.rs`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, FocusHandle, InteractiveElement, IntoElement, ListOffset, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, px, relative,
};

use super::outline_model::{
    BAR_HEIGHT, MIN_PROMPTS, Place, RIPPLE_SPAN, active_prompt, bar_lift, bar_stack, prompt_blocks,
    prompt_preview, stack_budget,
};
use super::{TranscriptView, row_turn};
use crate::app::BenCodeApp;
use crate::ui::motion::cubic_bezier;
use crate::ui::sidebar_popovers::popover_frame;

const OPEN_DELAY: Duration = Duration::from_millis(25);
const SCROLL_INSET: f32 = 8.0;
/// Rows above the target have no height until drawn; the inset is taken
/// once the frame at the target has measured them (as find does).
const INSET_SETTLE: Duration = Duration::from_millis(32);
const POPOVER_WIDTH: f32 = 288.0;
const BAR_WIDTH: f32 = 11.0;
const BAR_WIDTH_LIFTED: f32 = 24.0;
const BAR_OPACITY_IDLE: f32 = 0.15;
const BAR_OPACITY_LIT: f32 = 0.85;
const RIPPLE_STEP: Duration = Duration::from_millis(18);
const TRANSITION: Duration = Duration::from_millis(200);
/// MonoCode hides the rail in panes narrower than `58rem`.
const MIN_PANE_WIDTH: f32 = 928.0;
/// MonoCode `NEAR_END_PX`.
const NEAR_END: f32 = 16.0;
const RAIL_RIGHT: f32 = 16.0;
const CARD_GAP: f32 = 10.0;

/// A bar's width share (`lift`) and opacity.
type Look = (f32, f32);

/// MonoCode's `transition-[width,opacity] duration-200 ease-out`, each bar
/// a beat later the farther it is from the hovered one.
#[derive(Default)]
struct BarMotion {
    /// What drove the last targets: the hovered and the lit prompt.
    key: (Option<String>, Option<String>),
    since: Option<Instant>,
    from: HashMap<String, Look>,
    to: HashMap<String, (Look, Duration)>,
}

impl BarMotion {
    fn look(&self, id: &str, now: Instant) -> Look {
        let Some(((lift, opacity), delay)) = self.to.get(id).copied() else {
            return (0.0, BAR_OPACITY_IDLE);
        };
        let (from_lift, from_opacity) = self
            .from
            .get(id)
            .copied()
            .unwrap_or((0.0, BAR_OPACITY_IDLE));
        let elapsed = self.since.map_or(TRANSITION + delay, |since| {
            now.saturating_duration_since(since)
        });
        let t = elapsed.checked_sub(delay).map_or(0.0, |run| {
            (run.as_secs_f32() / TRANSITION.as_secs_f32()).min(1.0)
        });
        let eased = cubic_bezier(0.0, 0.0, 0.58, 1.0)(t);
        (
            from_lift + (lift - from_lift) * eased,
            from_opacity + (opacity - from_opacity) * eased,
        )
    }

    /// Retargets the bars, starting each from where it is now.
    fn retarget(
        &mut self,
        key: (Option<String>, Option<String>),
        to: HashMap<String, (Look, Duration)>,
    ) {
        let now = Instant::now();
        if self.since.is_none() {
            // The first frame draws the bars where they belong.
            self.from = to
                .iter()
                .map(|(id, (look, _))| (id.clone(), *look))
                .collect();
        } else {
            self.from = to
                .keys()
                .map(|id| (id.clone(), self.look(id, now)))
                .collect();
        }
        self.to = to;
        self.key = key;
        self.since = Some(now);
    }

    fn animating(&self, now: Instant) -> bool {
        let longest = self
            .to
            .values()
            .map(|(_, delay)| *delay)
            .max()
            .unwrap_or_default();
        self.since
            .is_some_and(|since| now.saturating_duration_since(since) < TRANSITION + longest)
    }
}

/// One transcript's outline state (MonoCode `PromptOutline`'s hooks).
#[derive(Default)]
pub struct OutlineState {
    focus: Option<FocusHandle>,
    /// The hovered bar's prompt block id; the keyboard moves it too.
    hover: Option<String>,
    /// The card is open (after `OPEN_DELAY`).
    open: bool,
    open_pending: bool,
    pointer_inside: bool,
    /// The rail holds keyboard focus (`:focus-visible`).
    keyboard: bool,
    /// The bar the keyboard is on (MonoCode `focusId`).
    cursor: Option<String>,
    /// The bars drawn last, by prompt block id, for the keyboard.
    bars: Vec<String>,
    /// The prompt in view as last drawn.
    active: Option<String>,
    motion: BarMotion,
}

/// The rows of each prompt's turn start, in prompt order.
fn prompt_rows(view: &TranscriptView, prompts: &[usize]) -> Vec<Option<usize>> {
    prompts
        .iter()
        .map(|block| {
            let turn = view.turns.iter().position(|t| t.user == Some(*block))?;
            view.rows.iter().position(|row| row_turn(row) == Some(turn))
        })
        .collect()
}

/// Where each prompt's row lies against the list's viewport as last laid
/// out. Rows before the scroll top are above; unmeasured ones are below.
fn places(view: &TranscriptView, rows: &[Option<usize>]) -> Vec<Place> {
    let viewport = view.list.viewport_bounds();
    let top = view.list.logical_scroll_top().item_ix;
    rows.iter()
        .map(|row| {
            let Some(row) = *row else {
                return Place::Below;
            };
            if row < top {
                return Place::Above;
            }
            match view.list.bounds_for_item(row) {
                Some(b) if b.bottom() <= viewport.top() => Place::Above,
                Some(b) if b.top() < viewport.bottom() => Place::Inside,
                _ => Place::Below,
            }
        })
        .collect()
}

/// MonoCode's `distanceToEnd <= NEAR_END_PX`: the list follows its tail,
/// holds the sent prompt, or its last row ends within 16px of the bottom.
fn near_end(view: &TranscriptView) -> bool {
    let list = &view.list;
    if list.is_following_tail() || view.holds_prompt() {
        return true;
    }
    let Some(last) = view.rows.len().checked_sub(1) else {
        return true;
    };
    list.bounds_for_item(last)
        .is_some_and(|b| b.bottom() <= list.viewport_bounds().bottom() + px(NEAR_END))
}

impl BenCodeApp {
    /// The rail over `session_id`'s transcript, when it has two prompts or
    /// more and the pane is wide enough.
    pub fn render_prompt_outline(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        let prompts = prompt_blocks(&session.blocks);
        if prompts.len() < MIN_PROMPTS {
            return None;
        }
        let ids: Vec<String> = prompts
            .iter()
            .map(|ix| session.blocks[*ix].id.clone())
            .collect();
        let view = self.transcripts.get_mut(session_id)?;
        let viewport = view.list.viewport_bounds();
        // A hidden pane has a zero-size box; the rule would pick the last prompt.
        if viewport.size.height <= px(0.0) || f32::from(viewport.size.width) < MIN_PANE_WIDTH {
            return None;
        }
        let rows = prompt_rows(view, &prompts);
        let near_end = near_end(view);
        let active = active_prompt(&places(view, &rows), near_end);
        let stack = bar_stack(
            ids.len(),
            active,
            stack_budget(f32::from(viewport.size.height)),
        );
        let bars: Vec<String> = ids[stack.range.clone()].to_vec();
        let active_id = active.map(|a| ids[a].clone());

        let state = &mut view.outline;
        let focus = state
            .focus
            .get_or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let hover_ix = state
            .hover
            .as_ref()
            .and_then(|h| bars.iter().position(|b| b == h));
        // The pointer owns the fill while it is on the rail; off it, the
        // fill marks the scroll position.
        let lit_id = match hover_ix {
            Some(ix) => Some(bars[ix].clone()),
            None => active_id.clone(),
        };
        let key = (hover_ix.map(|ix| bars[ix].clone()), lit_id.clone());
        if state.motion.key != key || state.motion.since.is_none() || state.bars != bars {
            let to = bars
                .iter()
                .enumerate()
                .map(|(ix, id)| {
                    let distance = hover_ix.map_or(0, |h| ix.abs_diff(h)).min(RIPPLE_SPAN);
                    let opacity = if lit_id.as_deref() == Some(id) {
                        BAR_OPACITY_LIT
                    } else {
                        BAR_OPACITY_IDLE
                    };
                    let look = (bar_lift(ix, hover_ix), opacity);
                    (id.clone(), (look, RIPPLE_STEP * distance as u32))
                })
                .collect();
            state.motion.retarget(key, to);
        }
        state.bars = bars.clone();
        state.active = active_id;
        let now = Instant::now();
        let looks: Vec<Look> = bars.iter().map(|id| state.motion.look(id, now)).collect();
        let animating = state.motion.animating(now);
        let card = hover_ix.filter(|_| state.open).and_then(|ix| {
            prompt_preview(&session.blocks, prompts[stack.range.start + ix]).map(|p| (ix, p))
        });
        let keyboard = state.keyboard;

        let fg = cx.theme().colors.fg;
        let step = BAR_HEIGHT + stack.gap;
        let sid = session_id.to_string();
        let mut rail = div()
            .id(SharedString::from(format!("prompt-outline-{session_id}")))
            .track_focus(&focus)
            .key_context("PromptOutline")
            .relative()
            .flex()
            .flex_col()
            .items_end()
            .w(px(BAR_WIDTH_LIFTED))
            .on_hover(cx.listener({
                let sid = sid.clone();
                move |this, hovered: &bool, _, cx| this.outline_pointer(&sid, *hovered, cx)
            }));
        for (id, (lift, opacity)) in bars.iter().zip(looks) {
            let (hover_sid, hover_id, jump_sid, jump_id) =
                (sid.clone(), id.clone(), sid.clone(), id.clone());
            rail = rail.child(
                div()
                    .id(SharedString::from(format!("prompt-bar-{session_id}-{id}")))
                    .flex()
                    .flex_none()
                    .w_full()
                    .h(px(step))
                    .items_center()
                    .justify_end()
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hover_outline_bar(&hover_sid, &hover_id, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.jump_to_prompt(&jump_sid, &jump_id, cx)
                    }))
                    .child(
                        div()
                            .h(px(BAR_HEIGHT))
                            .w(px(BAR_WIDTH + (BAR_WIDTH_LIFTED - BAR_WIDTH) * lift))
                            .rounded_full()
                            .bg(fg)
                            .opacity(opacity),
                    ),
            );
        }
        if let Some((ix, preview)) = card {
            let centre = ix as f32 * step + step / 2.0;
            let line =
                |text: String, color: gpui::Hsla| div().line_clamp(2).text_color(color).child(text);
            rail = rail.child(
                div()
                    .absolute()
                    .top(px(centre))
                    .right(px(BAR_WIDTH_LIFTED + CARD_GAP))
                    .h(px(0.0))
                    .flex()
                    .items_center()
                    .child(
                        popover_frame(cx)
                            .w(px(POPOVER_WIDTH))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .p_3()
                            .text_size(px(14.0))
                            .line_height(relative(1.375))
                            .child(line(preview.title, fg))
                            .children(preview.reply.map(|reply| line(reply, fg.opacity(0.45))))
                            .children(preview.detail.map(|detail| {
                                line(detail, fg.opacity(0.35))
                                    .border_l_2()
                                    .border_color(fg.opacity(0.15))
                                    .pl_3()
                            })),
                    ),
            );
        }
        // Keyboard focus (`:focus-visible`) holds the card; noticed at
        // prepaint, where the window is at hand. Animation asks for frames.
        let weak = cx.entity().downgrade();
        let probe = canvas(
            move |_, window, cx| {
                if animating {
                    window.request_animation_frame();
                }
                let now_keyboard = focus.is_focused(window) && window.last_input_was_keyboard();
                if now_keyboard != keyboard {
                    let (weak, sid) = (weak.clone(), sid.clone());
                    cx.defer(move |cx| {
                        if let Err(err) = weak
                            .update(cx, |this, cx| this.outline_keyboard(&sid, now_keyboard, cx))
                        {
                            log::debug!("outline focus after app drop: {err:#}");
                        }
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0();
        Some(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(RAIL_RIGHT))
                .flex()
                .items_center()
                .child(rail.child(probe))
                .into_any_element(),
        )
    }

    fn outline_state(&mut self, session_id: &str) -> Option<&mut OutlineState> {
        self.transcripts.get_mut(session_id).map(|v| &mut v.outline)
    }

    /// MonoCode `hoverBar`: the ripple follows the pointer at once; the
    /// card waits out a pass-through.
    fn hover_outline_bar(&mut self, session_id: &str, id: &str, cx: &mut Context<Self>) {
        let Some(state) = self.outline_state(session_id) else {
            return;
        };
        state.hover = Some(id.to_string());
        if !state.open && !state.open_pending {
            state.open_pending = true;
            let sid = session_id.to_string();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(OPEN_DELAY).await;
                let opened = this.update(cx, |this, cx| {
                    if let Some(state) = this.outline_state(&sid)
                        && std::mem::take(&mut state.open_pending)
                    {
                        state.open = state.hover.is_some();
                        cx.notify();
                    }
                });
                if let Err(err) = opened {
                    log::debug!("outline card after app drop: {err:#}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// MonoCode `close`.
    fn close_outline(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.outline_state(session_id) {
            state.hover = None;
            state.open = false;
            state.open_pending = false;
            cx.notify();
        }
    }

    /// MonoCode `leaveRail`: keyboard focus holds the card open after the
    /// pointer moves away.
    fn outline_pointer(&mut self, session_id: &str, inside: bool, cx: &mut Context<Self>) {
        let Some(state) = self.outline_state(session_id) else {
            return;
        };
        state.pointer_inside = inside;
        if !inside && !state.keyboard {
            self.close_outline(session_id, cx);
        }
    }

    /// The rail gained or lost keyboard focus (MonoCode's bar `onFocus`
    /// with `:focus-visible`, and `blurRail`).
    fn outline_keyboard(&mut self, session_id: &str, keyboard: bool, cx: &mut Context<Self>) {
        let Some(state) = self.outline_state(session_id) else {
            return;
        };
        if state.keyboard == keyboard {
            return;
        }
        state.keyboard = keyboard;
        if keyboard {
            // MonoCode `tabId`: the last bar walked to, else the prompt in
            // view, else the first bar.
            let tab = [state.cursor.clone(), state.active.clone()]
                .into_iter()
                .flatten()
                .find(|c| state.bars.contains(c))
                .or_else(|| state.bars.first().cloned());
            state.cursor = tab.clone();
            state.hover = tab;
            state.open = state.hover.is_some();
            cx.notify();
        } else if !state.pointer_inside {
            self.close_outline(session_id, cx);
        }
    }

    /// The session whose rail holds focus.
    fn focused_outline(&self, window: &Window) -> Option<String> {
        self.transcripts.iter().find_map(|(id, view)| {
            view.outline
                .focus
                .as_ref()
                .filter(|f| f.is_focused(window))
                .map(|_| id.clone())
        })
    }

    /// ↑ / ↓ on the rail (MonoCode `onKeyDown`).
    pub fn step_outline(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        let Some(sid) = self.focused_outline(window) else {
            return;
        };
        let Some(state) = self.outline_state(&sid) else {
            return;
        };
        let from = state
            .cursor
            .as_ref()
            .and_then(|c| state.bars.iter().position(|b| b == c))
            .unwrap_or(0);
        let Some(next) = from
            .checked_add_signed(delta)
            .and_then(|ix| state.bars.get(ix))
            .cloned()
        else {
            return;
        };
        state.cursor = Some(next.clone());
        state.hover = Some(next);
        state.open = true;
        state.keyboard = true;
        cx.notify();
    }

    /// Enter on the rail: jumps to the bar the keyboard is on.
    pub fn open_outline_cursor(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(sid) = self.focused_outline(window) else {
            return;
        };
        let cursor = self.outline_state(&sid).and_then(|s| s.cursor.clone());
        if let Some(id) = cursor {
            self.jump_to_prompt(&sid, &id, cx);
        }
    }

    /// MonoCode `jumpTo`: the prompt's turn to the top, 8px under the
    /// edge, and the list stops following the tail.
    pub fn jump_to_prompt(&mut self, session_id: &str, block_id: &str, cx: &mut Context<Self>) {
        let Some(block) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.blocks.iter().position(|b| b.id == block_id))
        else {
            return;
        };
        let Some(view) = self.transcripts.get(session_id) else {
            return;
        };
        let row = prompt_rows(view, &[block])[0].unwrap_or(0);
        view.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.0),
        });
        let list = view.list.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INSET_SETTLE).await;
            let settled = this.update(cx, |_, cx| {
                if list.logical_scroll_top().item_ix == row {
                    list.scroll_by(px(-SCROLL_INSET));
                    cx.notify();
                }
            });
            if let Err(err) = settled {
                log::debug!("outline jump after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }
}
