//! The agent's work inside a turn (MonoCode `WorkFoldLine`, `ActivityPhases`,
//! `ActivityToolRow`, `ActivityThinkingRow`): one status line per turn, work
//! grouped into phases titled by the agent's own words or a summary of the
//! calls, and one-line tool rows (verb, then the file or command), an
//! edit's with what it changed under it (`edit_patch.rs`).

use std::sync::Arc;
use std::time::Duration;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    Animation, AnimationExt, AnyElement, App, Context, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, div, prelude::*, relative,
};

use crate::ui::scale::px;
use crate::ui::spinner;

use super::blocks::markdown;
use super::markdown::Tone;
use super::mono;
use super::turns::{self, Item, Phase, ToolState, TurnLayout, WorkKind};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{Block, SessionRow};
use crate::ui::HarnessIcon;
use crate::ui::mascot::pixel_sprite;
use crate::ui::tokens::Text;
use crate::ui::transcript::selection::OwnsPress;

/// MonoCode fades a step in as it lands (`zen-step-in`).
const STEP_ENTRANCE: Duration = Duration::from_millis(180);
const ICON_BOX: f32 = 14.0;

fn muted(color: Hsla, opacity: f32) -> Hsla {
    color.opacity(opacity)
}

fn phase_icon(kind: WorkKind) -> Option<IconName> {
    match kind {
        WorkKind::Edit => Some(IconName::PenLine),
        WorkKind::Research => Some(IconName::Search),
        WorkKind::Run => Some(IconName::Terminal),
        WorkKind::Agent => Some(IconName::Bot),
        WorkKind::Other => Some(IconName::Wrench),
        WorkKind::Note => Some(IconName::Minus),
        WorkKind::Think => None,
    }
}

fn small_icon(icon: IconName, color: Hsla) -> AnyElement {
    Icon::new(icon)
        .size(IconSize::Xs)
        .color(color)
        .into_any_element()
}

/// The 14px slot that shows an icon, or a chevron on hover / when open.
fn icon_slot(icon: AnyElement, open: bool, expandable: bool, group: &str, cx: &App) -> AnyElement {
    let chevron = muted(cx.theme().colors.fg, 0.45);
    if open {
        return div()
            .flex_none()
            .size(px(ICON_BOX))
            .child(small_icon(IconName::ChevronDown, chevron))
            .into_any_element();
    }
    let group = SharedString::from(group.to_string());
    let hover_group = group.clone();
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(ICON_BOX))
        .child(
            div()
                .when(expandable, |el| {
                    el.group_hover(hover_group, |s| s.opacity(0.0))
                })
                .child(icon),
        )
        .when(expandable, |el| {
            el.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(0.0)
                    .group_hover(group, |s| s.opacity(1.0))
                    .child(small_icon(IconName::ChevronRight, chevron)),
            )
        })
        .into_any_element()
}

/// Text in the dim 14px voice of the work rows; brighter on hover.
fn dim_label(text: impl Into<SharedString>, group: &str, cx: &App) -> AnyElement {
    let fg = cx.theme().colors.fg;
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(Text::Lg.px())
        .text_color(muted(fg, 0.5))
        .group_hover(SharedString::from(group.to_string()), move |s| {
            s.text_color(muted(fg, 0.8))
        })
        .child(text.into())
        .into_any_element()
}

fn shimmer_label(text: String, cx: &App) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(Text::Lg.px())
        .child(spinner::shimmer(text, cx))
        .into_any_element()
}

fn gpui_tooltip(text: &'static str) -> impl Fn(&mut gpui::Window, &mut App) -> gpui::AnyView {
    bencode_ui::primitives::Tooltip::text(text)
}

/// The blocks a list of work rows draws from: the thread's own, or the
/// steps of a subagent's run standing in as blocks.
#[derive(Clone, Copy)]
pub(super) struct Trail<'a> {
    pub session_id: &'a str,
    pub cwd: &'a str,
    pub harness: &'a str,
    pub blocks: &'a [Arc<Block>],
    /// The thread block a run's text selects as: its Agent call.
    pub select_as: Option<usize>,
}

impl<'a> Trail<'a> {
    pub fn of(session: &'a SessionRow) -> Self {
        Self {
            session_id: &session.id,
            cwd: &session.cwd,
            harness: &session.harness,
            blocks: &session.blocks,
            select_as: None,
        }
    }

    fn select(&self, ix: usize) -> usize {
        self.select_as.unwrap_or(ix)
    }
}

impl BenCodeApp {
    /// The turn's status line: "Opus working for 12s" shimmering while the
    /// agent runs, "Opus worked for 1m 4s" after, toggling the folded work.
    pub(super) fn render_fold_line(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        cx: &Context<Self>,
    ) -> AnyElement {
        let blocks = &session.blocks;
        let turn_id = turn.id(blocks).to_string();
        let expandable = turn.fold.is_some();
        let open = expandable && self.transcript_ui.open_folds.contains(&turn_id);
        let group = format!("fold-{turn_id}");
        if turn.mono
            && let Some(mono) = self.mono_for_session(&session.id)
        {
            return self.render_mono_fold_line(session, turn, mono, open, cx);
        }
        let label = if turn.live {
            shimmer_label(self.live_status(session, turn), cx)
        } else {
            dim_label(settled_title(blocks, turn), &group, cx)
        };
        let harness = turn.harness(blocks).unwrap_or(&session.harness).to_string();
        let icon = HarnessIcon::new(&harness)
            .size(px(ICON_BOX))
            .into_any_element();
        let row = div()
            .id(SharedString::from(group.clone()))
            .group(SharedString::from(group.clone()))
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .px_4()
            .py_1()
            .child(icon_slot(icon, open, expandable, &group, cx))
            .child(label);
        if !expandable {
            return row.into_any_element();
        }
        row.cursor_pointer()
            .owns_press()
            .on_click(cx.listener(move |this, _, _, cx| {
                let folds = &mut this.transcript_ui.open_folds;
                if !folds.remove(&turn_id) {
                    folds.insert(turn_id.clone());
                }
                this.redraw_regions(
                    &[
                        crate::ui::region::Region::Chat,
                        crate::ui::region::Region::Mono,
                    ],
                    cx,
                );
            }))
            .into_any_element()
    }

    /// MonoCode `MonoTurnHeader`: a Mono signs its turn with its pill (the
    /// still mascot and its name), and while it works the status of the
    /// latest step runs beside it. The pill shows the turn's work on the
    /// Overview's Activity tab (MonoCode `onShowActivity`), and is marked
    /// while it does.
    fn render_mono_fold_line(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        mono: &crate::monos::Mono,
        open: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let blocks = &session.blocks;
        let turn_id = turn.id(blocks).to_string();
        let expandable = turn.fold.is_some() || turn.live;
        let group = format!("fold-{turn_id}");
        let fg = cx.theme().colors.fg;
        let shown = self.mono_activity_shown(&session.id) == Some(turn_id.as_str());
        let mark = pixel_sprite(
            &self.mono_mascot(mono).rest,
            px(ICON_BOX),
            self.mono_color(mono),
            false,
        );
        let pill = mono::signature_pill(
            icon_slot(mark, open || shown, expandable, &group, cx),
            mono::signature_name(mono.display_name(), cx),
            cx,
        );
        let ticker = turn.live.then(|| {
            let request = self.pending_permission_for(&session.id);
            let live = mono::Live {
                approval: request.is_some_and(|r| r.tool != crate::app::QUESTION_TOOL),
                question: request.is_some_and(|r| r.tool == crate::app::QUESTION_TOOL),
                background: self.background_tasks_for(&session.id).len(),
            };
            let steps: Vec<&Block> = turn.range.clone().map(|ix| &*blocks[ix]).collect();
            mono::work_status(&steps, live)
        });
        let row = div()
            .id(SharedString::from(group.clone()))
            .group(SharedString::from(group.clone()))
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .px_4()
            .py_1()
            .text_size(Text::Lg.px())
            .text_color(muted(fg, 0.5))
            .map(|el| match ticker {
                Some(status) => el
                    .child(div().flex().flex_none().max_w(relative(0.45)).child(pill))
                    .child(div().flex_none().text_color(muted(fg, 0.25)).child("·"))
                    .child(shimmer_label(status, cx)),
                None => el.child(pill),
            });
        if !expandable {
            return row.into_any_element();
        }
        let session_id = session.id.clone();
        row.cursor_pointer()
            .owns_press()
            .tooltip(gpui_tooltip(if shown {
                "Hide activity"
            } else {
                "Show activity"
            }))
            .on_click(
                cx.listener(move |this, _, _, cx| {
                    this.show_mono_activity(&session_id, &turn_id, cx)
                }),
            )
            .into_any_element()
    }

    /// "Waiting for answers" / "Waiting for approval", or the running clock.
    fn live_status(&self, session: &SessionRow, turn: &TurnLayout) -> String {
        if let Some(request) = self.pending_permission_for(&session.id) {
            return if request.tool == crate::app::QUESTION_TOOL {
                "Waiting for answers"
            } else {
                "Waiting for approval"
            }
            .to_string();
        }
        let blocks = &session.blocks;
        let elapsed = turn.started_at(blocks).map(|start| now_ms() - start);
        let working = turns::working_duration(elapsed, turn.model_name(blocks), false);
        // Yielding with work still going is not the end of the turn: the
        // clock keeps running and the line says what it waits on.
        match self.background_tasks_for(&session.id).len() {
            0 => working,
            1 => format!("{working} · running in background"),
            n => format!("{working} · {n} tasks running in background"),
        }
    }

    /// An item of a turn: activity renders as phases, blocks as themselves.
    pub(super) fn render_turn_item(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        item: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        match &turn.items[item] {
            Item::Activity(group) => {
                let blocks = &session.blocks;
                if turn.live && turns::is_initial_thinking(blocks, &turn.items, item) {
                    return initial_thinking(cx);
                }
                let done = turn.activity_done(blocks, item);
                div()
                    .px_4()
                    .child(self.render_phase_list(Trail::of(session), group, done, cx))
                    .into_any_element()
            }
            Item::Subagents(group) => div()
                .px_4()
                .child(self.render_subagent_stack(Trail::of(session), group, turn.live, cx))
                .into_any_element(),
            Item::Block(ix) => {
                let under_work = item > 0
                    && matches!(
                        turn.items.get(item - 1),
                        Some(Item::Activity(_) | Item::Subagents(_))
                    );
                let live = turn.live && *ix + 1 == session.blocks.len();
                self.render_block(session, *ix, live, under_work, cx)
            }
        }
    }

    /// An item inside an open fold, on the fold's rail.
    pub(super) fn render_fold_item(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        item: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let body = match &turn.items[item] {
            Item::Activity(group) => self.render_phase_list(Trail::of(session), group, true, cx),
            Item::Subagents(group) => {
                self.render_subagent_stack(Trail::of(session), group, false, cx)
            }
            Item::Block(ix) => {
                let text = turns::text(&session.blocks[*ix]).to_string();
                let id = SharedString::from(format!("{}-fold-{ix}", session.id));
                div()
                    .py_1()
                    .child(markdown(
                        id,
                        &text,
                        None,
                        Tone::Fold,
                        self.seg_ctx(&session.id, *ix, false, cx),
                        cx,
                    ))
                    .into_any_element()
            }
        };
        div()
            .px_4()
            .child(
                div()
                    .ml(px(7.0))
                    .pl(px(12.0))
                    .pb_1()
                    .border_l_1()
                    .border_color(muted(fg, 0.14))
                    .child(body),
            )
            .into_any_element()
    }

    pub(super) fn render_phase_list(
        &self,
        trail: Trail<'_>,
        group: &[usize],
        done: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let phases = turns::build_phases(trail.blocks, group);
        let last = phases.len().saturating_sub(1);
        // Blocks, not flex columns: a run's trail nests a phase list inside
        // another, and every flex level multiplies the layout passes.
        div()
            .min_w_0()
            .children(phases.iter().enumerate().map(|(ix, phase)| {
                div().when(ix > 0, |el| el.mt_1()).child(self.render_phase(
                    trail,
                    phase,
                    !done && ix == last,
                    cx,
                ))
            }))
            .into_any_element()
    }

    /// MonoCode `ActivityPhaseGroup`: open while it is the live phase, folded
    /// to its title once the agent moves on; click to open or close.
    fn render_phase(
        &self,
        trail: Trail<'_>,
        phase: &Phase,
        active: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        if phase.headline.is_none() && phase.steps.len() == 1 {
            // A lone call the agent never introduced is not a group. A run
            // brings its own mascot.
            let icon = phase_icon(phase.kind)
                .filter(|_| !turns::is_subagent(&trail.blocks[phase.steps[0]]));
            return div()
                .flex()
                .min_w_0()
                .items_start()
                .gap_1p5()
                .when_some(icon, |el, icon| {
                    el.child(div().mt(px(7.0)).child(small_icon(icon, muted(fg, 0.45))))
                })
                .child(div().flex_1().min_w_0().child(self.render_step(
                    trail,
                    phase.steps[0],
                    active,
                    cx,
                )))
                .into_any_element();
        }
        let open = self
            .transcript_ui
            .phase_open
            .get(&phase.id)
            .copied()
            .unwrap_or(active);
        let title = turns::phase_title(trail.blocks, phase, active);
        let group = format!("phase-{}", phase.id);
        let label = if active {
            shimmer_label(title, cx)
        } else {
            dim_label(title, &group, cx)
        };
        let icon = phase_icon(phase.kind).map_or_else(
            || div().into_any_element(),
            |i| small_icon(i, muted(fg, 0.45)),
        );
        let phase_id = phase.id.clone();
        let header = div()
            .id(SharedString::from(group.clone()))
            .group(SharedString::from(group.clone()))
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .py_1()
            .cursor_pointer()
            .owns_press()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.transcript_ui
                    .phase_open
                    .insert(phase_id.clone(), !open);
                this.transcript_ui.reflow += 1;
                this.redraw_regions(
                    &[
                        crate::ui::region::Region::Chat,
                        crate::ui::region::Region::Mono,
                    ],
                    cx,
                );
            }))
            .child(icon_slot(icon, false, true, &group, cx))
            .child(label);
        div()
            .min_w_0()
            .child(header)
            .when(open, |el| {
                el.child(
                    div()
                        .ml(px(7.0))
                        .pl(px(12.0))
                        .border_l_1()
                        .border_color(muted(fg, 0.14))
                        .children(
                            phase
                                .steps
                                .iter()
                                .map(|&ix| self.render_step(trail, ix, active, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    /// One step of the work: a tool call, a thought, a line of prose. Steps
    /// that land while the phase is live fade in.
    fn render_step(
        &self,
        trail: Trail<'_>,
        ix: usize,
        live: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &trail.blocks[ix];
        // A settled turn's runs are work like any call, but still open onto
        // their own trail.
        let row = if turns::is_subagent(block) {
            self.render_subagent_row(trail, ix, live, cx)
        } else if turns::is_tool(block) {
            self.render_tool_row(trail, ix, live, cx)
        } else if turns::is_thinking(block) || turns::is_prose(block) {
            self.render_thought_row(trail, ix, cx)
        } else {
            dim_label(turns::text(block).to_string(), "status", cx)
        };
        // The fade runs from when the step landed, not from when it was
        // first drawn (MonoCode `PhaseStep`'s `turn`, set only on the render
        // a step arrives in): a phase opened again, or a row back in view,
        // draws its steps anew and must not fade them in a second time.
        let Some(entered) = live.then(|| step_entrance(block, now_ms())).flatten() else {
            return row;
        };
        div()
            .id(SharedString::from(format!("step-{}", block.id)))
            .child(row)
            .with_animation(
                SharedString::from(format!("step-in-{}", block.id)),
                Animation::new(STEP_ENTRANCE),
                move |el, _| el.opacity(entered),
            )
            .into_any_element()
    }

    /// MonoCode `ActivityThinkingRow` / `ActivityNoteRow`: one dim line,
    /// opening onto the full text.
    fn render_thought_row(&self, trail: Trail<'_>, ix: usize, cx: &Context<Self>) -> AnyElement {
        let block = &trail.blocks[ix];
        let key = block.id.clone();
        let open = self.expanded_reasoning.contains(&key);
        let summary = turns::prose_summary(turns::text(block));
        let summary = if summary.is_empty() {
            "Thinking".to_string()
        } else {
            summary
        };
        let group = format!("thought-{key}");
        let body = turns::text(block).to_string();
        let header = div()
            .id(SharedString::from(group.clone()))
            .group(SharedString::from(group.clone()))
            .flex()
            .min_w_0()
            .items_center()
            .py_1()
            .cursor_pointer()
            .owns_press()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded_reasoning.remove(&key) {
                    this.expanded_reasoning.insert(key.clone());
                }
                this.transcript_ui.reflow += 1;
                this.redraw_regions(
                    &[
                        crate::ui::region::Region::Chat,
                        crate::ui::region::Region::Mono,
                    ],
                    cx,
                );
            }))
            .child(dim_label(summary, &group, cx));
        div()
            .min_w_0()
            .child(header)
            .when(open, |el| {
                el.child(div().pb_2().child(markdown(
                    SharedString::from(format!("{group}-body")),
                    &body,
                    None,
                    Tone::Reasoning,
                    self.seg_ctx(trail.session_id, trail.select(ix), false, cx),
                    cx,
                )))
            })
            .into_any_element()
    }

    /// MonoCode `ActivityToolRow`: the verb dim, then the target in a file
    /// chip or monospace; a spinner while running, a red X when it failed.
    fn render_tool_row(
        &self,
        trail: Trail<'_>,
        ix: usize,
        live: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &trail.blocks[ix];
        let colors = &cx.theme().colors;
        let state = turns::tool_state(block);
        let failed = state == ToolState::Failed;
        let (verb, target, file) = tool_label(block, trail.cwd);
        let key = block.id.clone();
        let detail = turns::tool_detail(block)
            .filter(|_| failed)
            .map(str::to_string);
        let error_open = self.transcript_ui.open_tool_errors.contains(&key);
        let patch = (!failed)
            .then(|| self.render_edit_patch(block, file.as_deref(), cx))
            .flatten();
        let target_color = if failed {
            colors.danger
        } else {
            muted(colors.fg, 0.7)
        };
        let mono = cx.theme().mono_family.clone();
        let row = div()
            .id(SharedString::from(format!("tool-row-{key}")))
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .py_1()
            .when(detail.is_some(), |el| {
                let key = key.clone();
                el.cursor_pointer()
                    .owns_press()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let errors = &mut this.transcript_ui.open_tool_errors;
                        if !errors.remove(&key) {
                            errors.insert(key.clone());
                        }
                        this.transcript_ui.reflow += 1;
                        this.redraw_regions(
                            &[
                                crate::ui::region::Region::Chat,
                                crate::ui::region::Region::Mono,
                            ],
                            cx,
                        );
                    }))
            })
            .when_some(verb, |el, verb| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(Text::Lg.px())
                        .text_color(if failed {
                            colors.danger
                        } else {
                            muted(colors.fg, 0.5)
                        })
                        .child(verb),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .font_family(mono.clone())
                    .text_size(Text::Md.px())
                    .text_color(target_color)
                    .child(self.tool_target_el(&key, target, file, cx)),
            )
            .children(tool_status(state, live, cx));
        div()
            .min_w_0()
            .child(row)
            .when_some(detail.filter(|_| error_open), |el, detail| {
                el.child(
                    div()
                        .py_1()
                        .font_family(mono)
                        .text_size(Text::Base.px())
                        .line_height(px(20.0))
                        .text_color(colors.danger.opacity(0.8))
                        .child(detail),
                )
            })
            .children(patch)
            .into_any_element()
    }

    /// A file target is a chip that opens the file; others are plain text.
    fn tool_target_el(
        &self,
        key: &str,
        target: String,
        file: Option<String>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let Some(path) = file else {
            return div().min_w_0().truncate().child(target).into_any_element();
        };
        let accent = colors.accent;
        let hover_bg = muted(colors.fg, 0.1);
        div()
            .id(SharedString::from(format!("tool-file-{key}")))
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1()
            .rounded(px(4.0))
            .bg(muted(colors.fg, 0.06))
            .cursor_pointer()
            .owns_press()
            .hover(move |s| s.bg(hover_bg).text_color(accent))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_file_in_editor(&path, window, cx)),
            )
            .child(small_icon(IconName::FileText, muted(colors.fg, 0.5)))
            .child(div().min_w_0().truncate().child(target))
            .into_any_element()
    }
}

/// Trailing status: spinner while live, dashed circle while waiting, red X
/// on failure; success has none (MonoCode `ToolCallStatusIcon`).
fn tool_status(state: ToolState, live: bool, cx: &App) -> Option<AnyElement> {
    let colors = &cx.theme().colors;
    match state {
        ToolState::Pending if live => Some(spinner::ring(IconSize::Xs, cx)),
        ToolState::Pending => Some(small_icon(IconName::CircleDashed, muted(colors.fg, 0.4))),
        ToolState::Failed => Some(small_icon(IconName::X, colors.danger)),
        ToolState::Done => None,
    }
}

/// How far through its fade a step that landed at `started_at` is at `now`,
/// or `None` once it has settled (and for a step with no arrival time).
fn step_entrance(block: &Block, now: i64) -> Option<f32> {
    let since = now - block.started_at?;
    let whole = STEP_ENTRANCE.as_millis() as i64;
    // A clock set back shows the step rather than hiding it until then.
    (0..whole)
        .contains(&since)
        .then(|| since as f32 / whole as f32)
}

/// "Opus worked for 1m 4s", or what the folded work adds up to.
fn settled_title(blocks: &[Arc<Block>], turn: &TurnLayout) -> String {
    let model = turn.model_name(blocks);
    match turn.duration_ms(blocks) {
        Some(ms) => turns::working_duration(Some(ms), model, true),
        None => turns::work_summary(blocks, &folded_blocks(turn), false),
    }
}

/// The blocks a turn's fold summarises.
fn folded_blocks(turn: &TurnLayout) -> Vec<usize> {
    let Some((start, end)) = turn.fold else {
        return Vec::new();
    };
    turn.items[start..=end]
        .iter()
        .flat_map(|item| match item {
            Item::Activity(ixs) | Item::Subagents(ixs) => ixs.clone(),
            Item::Block(ix) => vec![*ix],
        })
        .collect()
}

/// `path` relative to `cwd` when it lives there.
fn display_path(path: &str, cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// MonoCode `composeToolTitle` reduced to BenCode's tool kinds: the verb, the
/// target, and the file to open when the target is one.
pub(crate) fn tool_label(
    block: &Block,
    cwd: &str,
) -> (Option<&'static str>, String, Option<String>) {
    let target = turns::tool_target(block);
    match turns::tool_kind_name(block) {
        "read" => (
            Some("Read"),
            display_path(target, cwd),
            Some(target.to_string()),
        ),
        "edit" => (
            Some("Edit"),
            display_path(target, cwd),
            Some(target.to_string()),
        ),
        "search" => (Some("Find"), target.to_string(), None),
        "skill" => (Some("Skill"), target.to_string(), None),
        _ if target.is_empty() => (None, "Working".to_string(), None),
        _ => (
            None,
            target.lines().next().unwrap_or(target).to_string(),
            None,
        ),
    }
}

/// MonoCode `InitialThinking`: "Thinking…" shimmering before any output.
fn initial_thinking(cx: &App) -> AnyElement {
    div()
        .px_4()
        .pt_3()
        .pb_1()
        .text_size(Text::Lg.px())
        .child(spinner::shimmer("Thinking…", cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_labels_split_verb_and_target() {
        let mut block = Block::new("t", "tool", "");
        block.tool = Some(serde_json::json!({"kind": "read", "title": "/repo/src/a.rs"}));
        let (verb, target, file) = tool_label(&block, "/repo");
        assert_eq!((verb, target.as_str()), (Some("Read"), "src/a.rs"));
        assert_eq!(file.as_deref(), Some("/repo/src/a.rs"));
        block.tool = Some(serde_json::json!({"kind": "execute", "title": "cargo test\nmore"}));
        assert_eq!(tool_label(&block, "/repo").1, "cargo test");
    }

    #[test]
    fn a_step_fades_in_only_while_it_lands() {
        let mut block = Block::new("t", "tool", "");
        assert_eq!(step_entrance(&block, 1_000), None);
        block.started_at = Some(1_000);
        assert_eq!(step_entrance(&block, 1_000), Some(0.0));
        assert_eq!(step_entrance(&block, 1_090), Some(0.5));
        // Drawn again later (its phase reopened, its row back in view).
        assert_eq!(step_entrance(&block, 1_180), None);
        assert_eq!(step_entrance(&block, 60_000), None);
        assert_eq!(step_entrance(&block, 900), None);
    }
}
