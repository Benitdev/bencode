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
    ParentElement, Pixels, SharedString, Styled, div, prelude::*, relative,
};

use crate::ui::baseline::{self, Line};
use crate::ui::scale::{px, ui_scale};
use crate::ui::spinner;

use super::blocks::markdown;
use super::bot;
use super::markdown::Tone;
use super::turns::{self, Item, Phase, ToolState, TurnLayout, WorkKind};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{Block, SessionRow};
use crate::ui::HarnessIcon;
use crate::ui::tokens::Text;
use crate::ui::transcript::selection::OwnsPress;

/// MonoCode fades a step in as it lands (`zen-step-in`).
const STEP_ENTRANCE: Duration = Duration::from_millis(180);
const ICON_BOX: f32 = 14.0;
/// `IconSize::Xs`, the rows' small icons, at 100%.
const SMALL_ICON: f32 = 12.0;
/// The text a row's leading icon lines up with: the tallest in the row,
/// so its line box starts at the row's top.
#[derive(Clone, Copy)]
struct RowText {
    size: Text,
    mono: bool,
    /// The line height at 100%, or `None` for GPUI's default.
    line: Option<f32>,
}

impl RowText {
    /// The work rows' dim label (`dim_label`, a tool's verb).
    const LABEL: Self = Self {
        size: Text::Lg,
        mono: false,
        line: None,
    };
    /// A tool's target, in a row with no verb.
    const TARGET: Self = Self {
        size: Text::Md,
        mono: true,
        line: None,
    };
    /// A Bot's name in its pill (`bot::signature_name`).
    const BOT_NAME: Self = Self {
        size: Text::Lg,
        mono: false,
        line: Some(20.0),
    };
}

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

/// How far below the row's top an icon `height` tall (at 100%) starts, in
/// line with `text`, whose line box starts at the row's top
/// (`ui::baseline`: centred, it drifted a pixel from one scale to the next).
fn icon_top(text: RowText, height: f32, cx: &App) -> Pixels {
    let theme = cx.theme();
    let family = if text.mono {
        &theme.mono_family
    } else {
        &theme.font_family
    };
    let size = text.size.px();
    let line = Line {
        family,
        size,
        height: text.line.map_or(size * baseline::PHI, px),
    };
    line.icon_top(baseline::round(line.baseline(cx)), px(height))
}

/// The 14px-wide slot for a row's leading icon, `height` tall at 100%: in
/// line with the `ml(7px)` fold rail and, vertically, with `text`.
fn icon_box(icon: AnyElement, height: f32, text: RowText, cx: &App) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .self_start()
        .mt(icon_top(text, height, cx))
        .justify_center()
        .w(px(ICON_BOX))
        .h(px(height))
        .child(icon)
}

/// The 14px slot that shows an icon, or a chevron on hover / when open.
fn icon_slot(
    icon: AnyElement,
    open: bool,
    expandable: bool,
    group: &str,
    text: RowText,
    cx: &App,
) -> AnyElement {
    let chevron = muted(cx.theme().colors.fg, 0.45);
    if open {
        return icon_box(
            small_icon(IconName::ChevronDown, chevron),
            SMALL_ICON,
            text,
            cx,
        )
        .into_any_element();
    }
    let group = SharedString::from(group.to_string());
    let hover_group = group.clone();
    icon_box(
        div()
            .when(expandable, |el| {
                el.group_hover(hover_group, |s| s.opacity(0.0))
            })
            .child(icon)
            .into_any_element(),
        ICON_BOX,
        text,
        cx,
    )
    .relative()
    .when(expandable, |el| {
        // The chevron a whole number of pixels into the slot, so it
        // rounds as the slot does.
        let inset = ((ICON_BOX - SMALL_ICON) / 2.0 * ui_scale()).round();
        el.child(
            div()
                .absolute()
                .top(gpui::px(inset))
                .left_0()
                .flex()
                .justify_center()
                .w_full()
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
        if turn.bot
            && let Some(bot) = self.bot_for_session(&session.id)
        {
            return self.render_bot_fold_line(session, turn, bot, open, cx);
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
            .child(icon_slot(
                icon,
                open,
                expandable,
                &group,
                RowText::LABEL,
                cx,
            ))
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
                        crate::ui::region::Region::Bot,
                    ],
                    cx,
                );
            }))
            .into_any_element()
    }

    /// MonoCode `MonoTurnHeader`: a Bot signs its turn with its pill (the
    /// still figure and its name), and while it works the status of the
    /// latest step runs beside it. The pill shows the turn's work on the
    /// Overview's Activity tab (MonoCode `onShowActivity`), and is marked
    /// while it does.
    fn render_bot_fold_line(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        bot: &crate::bots::Bot,
        open: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let blocks = &session.blocks;
        let turn_id = turn.id(blocks).to_string();
        let expandable = turn.fold.is_some() || turn.live;
        let group = format!("fold-{turn_id}");
        let fg = cx.theme().colors.fg;
        let shown = self.bot_activity_shown(&session.id) == Some(turn_id.as_str());
        let mark = crate::ui::figure::figure_still(bot.figure(), ICON_BOX, self.bot_color(bot));
        let pill = bot::signature_pill(
            icon_slot(
                mark,
                open || shown,
                expandable,
                &group,
                RowText::BOT_NAME,
                cx,
            ),
            bot::signature_name(bot.display_name(), cx),
            cx,
        );
        let ticker = turn.live.then(|| {
            let request = self.pending_permission_for(&session.id);
            let live = bot::Live {
                approval: request.is_some_and(|r| r.tool != crate::app::QUESTION_TOOL),
                question: request.is_some_and(|r| r.tool == crate::app::QUESTION_TOOL),
                background: self.background_tasks_for(&session.id).len(),
            };
            let steps: Vec<&Block> = turn.range.clone().map(|ix| &*blocks[ix]).collect();
            bot::work_status(&steps, live)
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
                    this.show_bot_activity(&session_id, &turn_id, cx)
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
    /// `after_line`: the row just above is the turn's status line.
    pub(super) fn render_turn_item(
        &self,
        session: &SessionRow,
        turn: &TurnLayout,
        item: usize,
        after_line: bool,
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
                let under_work = after_line
                    || item > 0
                        && matches!(
                            turn.items.get(item - 1),
                            Some(Item::Activity(_) | Item::Subagents(_))
                        );
                let live = turn.live && *ix + 1 == session.blocks.len();
                self.render_block(session, *ix, live, turn.live, under_work, cx)
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
            // brings its own figure.
            let icon = phase_icon(phase.kind)
                .filter(|_| !turns::is_subagent(&trail.blocks[phase.steps[0]]));
            return self.render_step(trail, phase.steps[0], active, icon, cx);
        }
        let open = self
            .transcript_ui
            .phase_open
            .get(&phase.id)
            .copied()
            // A running group opens itself, unless Settings keeps them closed.
            .unwrap_or(active && !self.collapse_tool_groups);
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
                        crate::ui::region::Region::Bot,
                    ],
                    cx,
                );
            }))
            .child(icon_slot(icon, false, true, &group, RowText::LABEL, cx))
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
                                .map(|&ix| self.render_step(trail, ix, active, None, cx)),
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
        leading_icon: Option<IconName>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &trail.blocks[ix];
        // A settled turn's runs are work like any call, but still open onto
        // their own trail.
        let row = if turns::is_subagent(block) {
            self.render_subagent_row(trail, ix, live, cx)
        } else if turns::is_tool(block) {
            self.render_tool_row(trail, ix, live, leading_icon, cx)
        } else if turns::is_thinking(block) || turns::is_prose(block) {
            self.render_thought_row(trail, ix, leading_icon, cx)
        } else {
            let label = dim_label(turns::text(block).to_string(), "status", cx);
            match leading_icon {
                Some(icon) => div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .py_1()
                    .child(icon_box(
                        small_icon(icon, muted(cx.theme().colors.fg, 0.45)),
                        SMALL_ICON,
                        RowText::LABEL,
                        cx,
                    ))
                    .child(label)
                    .into_any_element(),
                None => label,
            }
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
    fn render_thought_row(
        &self,
        trail: Trail<'_>,
        ix: usize,
        leading_icon: Option<IconName>,
        cx: &Context<Self>,
    ) -> AnyElement {
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
        let fg = cx.theme().colors.fg;
        let header = div()
            .id(SharedString::from(group.clone()))
            .group(SharedString::from(group.clone()))
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
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
                        crate::ui::region::Region::Bot,
                    ],
                    cx,
                );
            }))
            .when_some(leading_icon, |el, icon| {
                el.child(icon_box(
                    small_icon(icon, muted(fg, 0.45)),
                    SMALL_ICON,
                    RowText::LABEL,
                    cx,
                ))
            })
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
        leading_icon: Option<IconName>,
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
        let mono_font = cx.theme().mono_family.clone();
        let row_text = if verb.is_some() {
            RowText::LABEL
        } else {
            RowText::TARGET
        };
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
                                crate::ui::region::Region::Bot,
                            ],
                            cx,
                        );
                    }))
            })
            .when_some(leading_icon, |el, icon| {
                el.child(icon_box(
                    small_icon(icon, muted(colors.fg, 0.45)),
                    SMALL_ICON,
                    row_text,
                    cx,
                ))
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
                    .font_family(mono_font.clone())
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
                        .font_family(mono_font)
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
