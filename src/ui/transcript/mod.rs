//! Active thread: header, a virtualized transcript and the composer.
//!
//! Only visible rows are laid out (`gpui::list`), and only the streaming tail
//! is re-measured, so a long thread stays cheap while tokens arrive.

mod activity;
pub mod blocks;
pub mod find;
mod review_card;
pub mod turns;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, FollowMode, FontWeight, InteractiveElement, IntoElement, ListAlignment,
    ListOffset, ListState, ParentElement, Pixels, SharedString, Styled, Window, canvas, div,
    prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::motion::cubic_bezier;
use blocks::MESSAGE_MAX_WIDTH;
use turns::{Row, TurnLayout};

/// Rows near the end that may still change height while an agent runs.
const LIVE_TAIL_ROWS: usize = 3;
const LIST_OVERDRAW: gpui::Pixels = px(600.0);
/// The spacer under a sent prompt before the list has ever been laid out.
const UNMEASURED_GAP: Pixels = px(2000.0);

/// MonoCode `riseIntoAnchor`: a sent prompt fades in while it slides from
/// the upper viewport to its row, then the rest of its turn fades up.
const PROMPT_RISE: Duration = Duration::from_millis(560);
const PROMPT_FADE: Duration = Duration::from_millis(480);
const PROMPT_REVEAL: Duration = Duration::from_millis(320);
/// Where the prompt starts, as a fraction of the viewport from the top.
const PROMPT_RISE_FROM: f32 = 0.3;
const PROMPT_REVEAL_LIFT: Pixels = px(10.0);

/// The heights of the last turn's rows as they were last painted.
type RowHeights = Rc<RefCell<HashMap<usize, Pixels>>>;

/// What is needed to size the spacer that stretches the last turn to the
/// viewport (MonoCode `.transcript-turn-anchor`).
#[derive(Clone)]
struct AnchorProbe {
    list: ListState,
    heights: RowHeights,
    /// The turn's rows, without the spacer after them.
    turn: Range<usize>,
}

impl AnchorProbe {
    /// The room the turn leaves in the viewport. Rows never painted count
    /// as nothing: they lie past a viewport the painted ones already fill.
    fn gap(&self) -> Pixels {
        let viewport = self.list.viewport_bounds().size.height;
        if viewport <= px(0.0) {
            return UNMEASURED_GAP;
        }
        let heights = self.heights.borrow();
        let turn: Pixels = self.turn.clone().filter_map(|ix| heights.get(&ix)).sum();
        (viewport - turn).max(px(0.0))
    }
}

/// Scroll and measurement state for the transcript list.
pub struct TranscriptView {
    pub list: ListState,
    pub session_id: Option<String>,
    /// The thread laid out as turns, and the list rows drawn from them.
    pub turns: Vec<TurnLayout>,
    pub rows: Vec<Row>,
    /// The block of the current find match, worked out once per frame
    /// rather than per row (each lookup scans the whole thread).
    pub find_block: Option<usize>,
    /// How many blocks match the find query, counted with `find_block`.
    pub find_count: usize,
    /// Whether the last sync saw the agent running, so the tail is measured
    /// once more when the turn settles (its text stops revealing).
    live: bool,
    /// The last turn's user block, to notice a send.
    prompt: Option<String>,
    /// Whether the last turn is stretched to the viewport so its prompt
    /// sits at the top. Set by a send and kept until the thread is closed,
    /// like MonoCode's `anchorTurn`.
    anchored: bool,
    /// The last turn's rows while it is stretched; the spacer comes next.
    anchor_rows: Option<Range<usize>>,
    /// The spacer's height.
    gap: Pixels,
    /// Whether the scroll top is held on the prompt. The spacer is sized
    /// from the last paint, so while the turn is shorter than the viewport
    /// the list is laid out from the prompt down: a stale spacer then only
    /// overflows below the fold instead of pushing the prompt off the top.
    hold: bool,
    heights: RowHeights,
    /// When the last prompt was sent, while it rises into place.
    rise: Option<Instant>,
    /// The session review card as last measured (`SessionReview::stamp`).
    review_stamp: u64,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW),
            session_id: None,
            turns: Vec::new(),
            rows: Vec::new(),
            find_block: None,
            find_count: 0,
            live: false,
            prompt: None,
            anchored: false,
            anchor_rows: None,
            gap: px(0.0),
            hold: false,
            heights: RowHeights::default(),
            rise: None,
            review_stamp: 0,
        }
    }
}

impl TranscriptView {
    fn anchor_probe(&self) -> Option<AnchorProbe> {
        Some(AnchorProbe {
            list: self.list.clone(),
            heights: self.heights.clone(),
            turn: self.anchor_rows.clone()?,
        })
    }

    /// Whether the prompt is held at the top, where the list is at its end
    /// whatever a stale spacer says.
    pub fn holds_prompt(&self) -> bool {
        self.hold
    }

    /// Sizes the spacer and keeps the prompt at the top of the viewport
    /// until its turn outgrows it; from then on the list follows the tail,
    /// as MonoCode pins the stretched turn to the bottom.
    fn anchor_turn(&mut self, sent: bool) {
        let Some(probe) = self.anchor_probe() else {
            self.hold = false;
            return;
        };
        let spacer = probe.turn.end;
        if sent {
            self.heights.borrow_mut().clear();
        }
        let gap = probe.gap();
        let top = self.list.logical_scroll_top();
        let at_prompt = top.item_ix == probe.turn.start && top.offset_in_item == px(0.0);
        let at_end = self.list.is_following_tail() || top.item_ix > spacer;
        if gap == px(0.0) {
            if self.hold && (at_prompt || at_end) {
                self.list.set_follow_mode(FollowMode::Tail);
            }
            self.hold = false;
        } else {
            // A reader who scrolled away is left alone until they return.
            self.hold = sent || at_end || (self.hold && at_prompt);
            if self.hold {
                self.list.scroll_to(ListOffset {
                    item_ix: probe.turn.start,
                    offset_in_item: px(0.0),
                });
            }
        }
        if gap != self.gap {
            self.gap = gap;
            self.list.remeasure_items(spacer..spacer + 1);
        }
    }

    /// How far below its place row `ix` is drawn and how opaque, while the
    /// sent prompt rises.
    fn rise_motion(&self, ix: usize) -> Option<(Pixels, f32)> {
        let turn = self.anchor_rows.as_ref().filter(|turn| turn.contains(&ix))?;
        let elapsed = self.rise?.elapsed();
        let progress = |of: Duration| (elapsed.as_secs_f32() / of.as_secs_f32()).min(1.0);
        if ix == turn.start {
            let from = self.list.viewport_bounds().size.height * PROMPT_RISE_FROM;
            let rise = cubic_bezier(0.22, 1.0, 0.36, 1.0)(progress(PROMPT_RISE));
            // The fade gets its own gentler curve (CSS `ease-out`).
            let fade = cubic_bezier(0.0, 0.0, 0.58, 1.0)(progress(PROMPT_FADE));
            return Some((from * (1.0 - rise), fade));
        }
        // The rest of the turn waits until the prompt lands.
        let Some(landed) = elapsed.checked_sub(PROMPT_RISE) else {
            return Some((px(0.0), 0.0));
        };
        let reveal = cubic_bezier(0.22, 1.0, 0.36, 1.0)(
            (landed.as_secs_f32() / PROMPT_REVEAL.as_secs_f32()).min(1.0),
        );
        Some((PROMPT_REVEAL_LIFT * (1.0 - reveal), reveal))
    }

    /// Asks for another frame when this one's paint changed what the
    /// spacer should be; it goes after the list so the rows are measured.
    pub fn anchor_check(&self) -> Option<impl IntoElement + use<>> {
        let probe = self.anchor_probe()?;
        let (gap, hold) = (self.gap, self.hold);
        Some(
            canvas(
                move |_, window, _| {
                    let next = probe.gap();
                    if (next - gap).abs() > px(0.5) || (hold && next == px(0.0)) {
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0(),
        )
    }
}

/// The turn a row belongs to; the trailer and spacer follow the last one.
fn row_turn(row: &Row) -> Option<usize> {
    match row {
        Row::Item { turn, .. }
        | Row::FoldLine { turn }
        | Row::FoldItem { turn, .. }
        | Row::Footer { turn } => Some(*turn),
        Row::Trailer | Row::Spacer => None,
    }
}

/// What the transcript remembers the user opened (MonoCode keeps these in
/// component state): folds, phases, failed tool output, long messages.
#[derive(Default)]
pub struct TranscriptUiState {
    pub open_folds: HashSet<String>,
    pub phase_open: HashMap<String, bool>,
    pub open_tool_errors: HashSet<String>,
    pub expanded_messages: HashSet<String>,
    /// Copy buttons showing their check.
    pub copied: HashSet<String>,
}

/// The fold line's clock ticks once a second while an agent runs.
const CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(1);

impl BenCodeApp {
    /// Redraws every second while any agent runs, so "working for 12s" ticks.
    pub fn start_clock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let ticked = this.update(cx, |app, cx| {
                    let countdown = app.tick_usage_limits(cx);
                    if app.is_agent_running() || countdown {
                        cx.notify();
                    }
                });
                if ticked.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Gets or creates the transcript view for a given session.
    pub fn transcript_view_for(&mut self, session_id: &str) -> &mut TranscriptView {
        self.transcripts.entry(session_id.to_string()).or_default()
    }

    /// Lays the thread out as turns and keeps the list's rows in step. Rows
    /// before the first change keep their measurements; the live tail is
    /// re-measured while the agent streams.
    pub fn sync_transcript_list_for(&mut self, session_id: &str) {
        let running = self.is_agent_running_in(session_id);
        let waiting = self.pending_permission_for(session_id).is_some();
        // The review card takes the trailer row once the turn has settled.
        let review = !waiting && self.session_review_shown(session_id);
        let review_stamp = self
            .checkpoints
            .reviews
            .get(session_id)
            .map_or(0, |review| review.stamp);
        let session = self.sessions.iter().find(|s| s.id == session_id);
        let (turns, mut rows) = match session {
            Some(s) => {
                let turns = turns::layout_turns(&s.blocks, running);
                let rows =
                    turns::build_rows(
                    &s.blocks,
                    &turns,
                    &self.transcript_ui.open_folds,
                    waiting || review,
                );
                (turns, rows)
            }
            None => (Vec::new(), Vec::new()),
        };
        let prompt = session.and_then(|s| Some(s.blocks[turns.last()?.user?].id.as_str()));
        let view = self.transcripts.entry(session_id.to_string()).or_default();
        let fresh = view.session_id.as_deref() != Some(session_id);
        // MonoCode stretches the last turn after a send, and on opening a
        // thread whose agent is at work.
        let sent = !fresh && prompt.is_some() && view.prompt.as_deref() != prompt;
        if view.prompt.as_deref() != prompt {
            view.prompt = prompt.map(str::to_string);
        }
        view.anchored |= sent || (fresh && running);
        let last = turns.len().saturating_sub(1);
        view.anchor_rows = rows
            .iter()
            .position(|row| row_turn(row) == Some(last))
            .filter(|_| view.anchored && prompt.is_some())
            .map(|start| start..rows.len());
        if view.anchor_rows.is_some() {
            rows.push(Row::Spacer);
        }
        let count = rows.len();
        if fresh {
            view.list.reset(count);
            view.list.set_follow_mode(FollowMode::Tail);
            view.list.scroll_to_end();
            view.session_id = Some(session_id.to_string());
        } else {
            let old = view.rows.len();
            let same = view
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            if same < old || count != old {
                view.list.splice(same..old, count - same);
            }
            if running || view.live {
                view.list
                    .remeasure_items(count.saturating_sub(LIVE_TAIL_ROWS)..count);
            } else if view.review_stamp != review_stamp
                && let Some(trailer) = rows.iter().rposition(|row| *row == Row::Trailer)
            {
                // The card gained or lost rows.
                view.list.remeasure_items(trailer..trailer + 1);
            }
        }
        view.review_stamp = review_stamp;
        // A thread opened on its first prompt starts there, not at the tail.
        let introduce = sent || (fresh && running && turns.len() == 1);
        view.anchor_turn(introduce);
        if introduce && view.anchor_rows.is_some() {
            view.rise = Some(Instant::now());
        } else if view
            .rise
            .is_some_and(|sent| sent.elapsed() >= PROMPT_RISE + PROMPT_REVEAL)
        {
            view.rise = None;
        }
        view.live = running;
        view.turns = turns;
        view.rows = rows;
    }

    pub fn render_transcript_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Delegate to multi-pane tree layout
        self.render_pane_tree(cx)
    }

    pub fn render_transcript_row_for(
        &mut self,
        session_id: &str,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.transcripts.get(session_id) else {
            return div().into_any_element();
        };
        let (Some(row), Some(session)) = (
            view.rows.get(ix),
            self.sessions.iter().find(|s| s.id == session_id),
        ) else {
            return div().into_any_element();
        };
        let turn_of = |t: usize| &view.turns[t];
        // Markdown cannot paint single words, so the row of the current find
        // match is tinted instead (user messages paint their words).
        let find_hit = view.find_block.is_some_and(|block| {
            session.blocks.get(block).is_some_and(|b| b.role != "user")
                && find::row_shows(row, &view.turns, block)
        });
        let content = match row {
            Row::Item { turn, item } => self.render_turn_item(session, turn_of(*turn), *item, cx),
            Row::FoldLine { turn } => self.render_fold_line(session, turn_of(*turn), cx),
            Row::FoldItem { turn, item, .. } => {
                self.render_fold_item(session, turn_of(*turn), *item, cx)
            }
            Row::Footer { turn } => self.render_turn_footer(session, turn_of(*turn), cx),
            Row::Trailer => self.render_trailer(session, cx),
            Row::Spacer => div().h(view.gap).into_any_element(),
        };
        // The stretched turn's rows report their height for the spacer.
        let measure = view
            .anchor_rows
            .as_ref()
            .filter(|turn| turn.contains(&ix))
            .map(|_| {
                let heights = view.heights.clone();
                canvas(
                    move |bounds, _, _| {
                        heights.borrow_mut().insert(ix, bounds.size.height);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            });
        let motion = view.rise_motion(ix);
        if motion.is_some() {
            window.request_animation_frame();
        }
        div()
            .relative()
            .when_some(motion, |el, (below, opacity)| {
                el.top(below).opacity(opacity)
            })
            .w_full()
            .flex()
            .justify_center()
            .px_2()
            .child(
                div()
                    .w_full()
                    .max_w(MESSAGE_MAX_WIDTH)
                    .flex()
                    .flex_col()
                    .when(find_hit, |el| {
                        el.rounded(px(8.0))
                            .bg(cx.theme().colors.accent.opacity(0.1))
                    })
                    .child(content),
            )
            .children(measure)
            .into_any_element()
    }

    /// The permission prompt, inline under the work like MonoCode's
    /// `ApprovalControls`: what the agent wants, then Allow / Deny.
    pub fn render_trailer(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let Some(request) = self
            .pending_permission_for(&session.id)
            .filter(|r| r.tool != crate::app::QUESTION_TOOL)
        else {
            // A question is answered in the composer's form.
            if self.session_review_shown(&session.id) {
                return self.render_session_review(&session.id, cx);
            }
            return div().into_any_element();
        };
        let colors = &cx.theme().colors;
        let (allow_id, deny_id) = (session.id.clone(), session.id.clone());
        let button = |id: &str, label: &'static str, primary: bool| {
            let (bg, fg, hover) = if primary {
                (colors.fg, colors.bg, colors.fg.opacity(0.8))
            } else {
                (
                    colors.fg.opacity(0.1),
                    colors.fg.opacity(0.7),
                    colors.fg.opacity(0.2),
                )
            };
            div()
                .id(SharedString::from(format!("{id}-{}", session.id)))
                .px_2p5()
                .py_0p5()
                .rounded(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .bg(bg)
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_4()
            .py_1()
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(colors.fg.opacity(0.7))
                    .child(format!("Allow {}?", request.tool)),
            )
            .when(!request.description.is_empty(), |el| {
                el.child(
                    div()
                        .font_family(cx.theme().mono_family.clone())
                        .text_size(px(12.0))
                        .text_color(colors.fg.opacity(0.5))
                        .child(request.description.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("perm-allow", "Allow", true).on_click(cx.listener(
                        move |this, _, _, cx| this.answer_permission(&allow_id, true, cx),
                    )))
                    .child(button("perm-deny", "Deny", false).on_click(cx.listener(
                        move |this, _, _, cx| this.answer_permission(&deny_id, false, cx),
                    ))),
            )
            .into_any_element()
    }
}
