//! The navigator's Agents section (MonoCode's "Working" card, made
//! BenCode's own), under the threads: the threads in flight or finished unseen,
//! across projects, those waiting on the user first. What a thread asks to
//! run is answered here (Allow / Deny) without leaving the thread in view;
//! a question opens its thread. It shows once two agents are live, or one
//! waits or works outside the project in view, lists four until expanded,
//! and a click jumps to the thread in its project. The list itself is
//! `app/live_agents.rs`.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, relative,
};

use super::project_name;
use super::widgets::figure_icon;
use crate::app::live_agents::{LiveAgent, format_live_elapsed};
use crate::app::{BenCodeApp, same_project_path};
use crate::ui::region::Region;
use crate::ui::scale::px;
use crate::ui::spinner::terminal_spinner;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

/// MonoCode `LIVE_AGENT_MIN` / `LIVE_AGENT_CAP`.
const LIVE_AGENT_MIN: usize = 2;
const LIVE_AGENT_CAP: usize = 4;
/// The most of the navigator's height the section takes, so the threads
/// above keep theirs; expanded (MonoCode `max-h-[45vh]`) it takes more.
const SECTION_SHARE: f32 = 0.35;
const EXPANDED_SHARE: f32 = 0.5;
/// The second line and the buttons start under the title, past the figure.
const DETAIL_INSET: f32 = 20.0;

/// What the section keeps between frames.
#[derive(Default)]
pub struct LiveAgentsUi {
    pub expanded: bool,
}

impl BenCodeApp {
    /// Whether the section has anything the project's threads above do not
    /// already show.
    fn live_agents_shown(&self, agents: &[LiveAgent]) -> bool {
        agents.len() >= LIVE_AGENT_MIN
            || agents.iter().any(|agent| {
                agent.needs_approval || !same_project_path(&agent.cwd, &self.current_cwd)
            })
    }

    /// The section, or nothing while the threads above say it all.
    pub(crate) fn render_live_agents(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let agents = self.live_agents();
        if agents.is_empty() || !self.live_agents_shown(&agents) {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let expanded = self.live_agents_ui.expanded;
        let extra = agents.len().saturating_sub(LIVE_AGENT_CAP);
        let shown = if expanded {
            agents.len()
        } else {
            agents.len() - extra
        };
        let waiting = agents.iter().filter(|agent| agent.needs_approval).count();
        let now = crate::app::now_ms();

        // As the Projects header: `px-3`, the label `px-1`.
        let header = div()
            .flex()
            .items_center()
            .gap_1p5()
            .px(px(16.0))
            .pt_1()
            .pb(px(4.0))
            .text_size(Text::Base.px())
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_color(fg.ink(Ink::Secondary))
                    .child("Agents"),
            )
            .when(waiting > 0, |el| {
                let amber = Status::Attention.color(fg);
                el.child(
                    div()
                        .flex_none()
                        .px(px(5.0))
                        .rounded(px(4.0))
                        .bg(amber.opacity(0.14))
                        .text_size(Text::Xs.px())
                        .text_color(amber)
                        .child(format!("{waiting} waiting")),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(agents.len().to_string()),
            );
        // Scrolls once the section reaches its share of the navigator.
        let list = div()
            .id("live-agents-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(1.0))
            .px_2()
            .overflow_y_scroll()
            .children(
                agents[..shown]
                    .iter()
                    .map(|agent| self.render_live_agent(agent, now, cx)),
            );
        let more = (extra > 0).then(|| {
            div()
                .id("live-agents-more")
                .group("live-agents-more")
                .flex()
                .mx_2()
                .items_center()
                .justify_center()
                .gap_1()
                .py(px(4.0))
                .rounded(px(6.0))
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Secondary))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.live_agents_ui.expanded = !this.live_agents_ui.expanded;
                    this.redraw_regions(&[Region::Rail], cx);
                }))
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .size(IconSize::Xs)
                    .color(fg.ink(Ink::Secondary))
                    .group_hover_color("live-agents-more", fg),
                )
                .child(if expanded {
                    "Show less".to_string()
                } else {
                    format!("{extra} more")
                })
        });
        Some(
            div()
                .flex()
                .flex_col()
                .flex_initial()
                .min_h_0()
                .max_h(relative(if expanded {
                    EXPANDED_SHARE
                } else {
                    SECTION_SHARE
                }))
                .pt_2()
                .pb_2()
                .border_t_1()
                .border_color(fg.fill(Fill::Line))
                .child(header)
                .child(crate::ui::scrollbar::Scrolled::new(
                    "live-agents-scrollbar",
                    list,
                ))
                .children(more)
                .into_any_element(),
        )
    }

    /// While the navigator is collapsed to nothing: the title bar's pill,
    /// how many agents are live, amber with how many wait on the user; a
    /// click brings the navigator back with its Agents section. The icon
    /// rail lists the agents themselves (`render_compact_agents`).
    pub(crate) fn render_agents_badge(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let agents = self.live_agents();
        if agents.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let waiting = agents.iter().filter(|agent| agent.needs_approval).count();
        let tint = if waiting > 0 {
            Status::Attention.color(fg)
        } else {
            colors.accent
        };
        let tip = match waiting {
            0 => format!("{} agents at work", agents.len()),
            _ => format!("{} agents, {waiting} waiting for you", agents.len()),
        };
        Some(
            div()
                .id("agents-badge-pill")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h(px(24.0))
                .px(px(7.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .bg(tint.opacity(0.12))
                .hover(move |s| s.bg(tint.opacity(0.2)))
                .text_size(Text::Sm.px())
                .text_color(tint)
                .child(Icon::new(IconName::Bot).size(IconSize::Sm).color(tint))
                .child(match waiting {
                    0 => format!("{} at work", agents.len()),
                    _ => format!("{waiting} waiting"),
                })
                .tooltip(Tooltip::text(tip))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.is_rail_open = true;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// What an agent does or asks, as its row's second line says it.
    pub(crate) fn live_agent_activity(&self, agent: &LiveAgent) -> String {
        let request = agent
            .needs_approval
            .then(|| self.pending_permission_for(&agent.id))
            .flatten();
        match request {
            Some(request) if request.tool == crate::app::QUESTION_TOOL => {
                "Has a question".to_string()
            }
            Some(request) if request.description.is_empty() => {
                format!("Allow {}?", request.tool)
            }
            Some(request) => format!("{}: {}", request.tool, request.description),
            None if agent.needs_approval => "Needs approval".to_string(),
            None if agent.done => "Done".to_string(),
            None => agent.activity.clone(),
        }
    }

    /// One agent: its project's figure, the title and elapsed time, then
    /// what it does (or asks) and its project; Allow / Deny under a
    /// request to run something, Answer under a question.
    fn render_live_agent(&self, agent: &LiveAgent, now: i64, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
        let seed = project_name(&agent.cwd);
        let project = self.settings.rail.label(&agent.cwd, seed).to_string();
        let elapsed = live_agent_elapsed(agent, now);
        let request = agent
            .needs_approval
            .then(|| self.pending_permission_for(&agent.id))
            .flatten();
        let question = request.is_some_and(|r| r.tool == crate::app::QUESTION_TOOL);
        let activity = self.live_agent_activity(agent);
        let live = !agent.needs_approval && !agent.done;
        let selected = self.selected_session_id.as_deref() == Some(agent.id.as_str());
        let tip = [
            Some(agent.title.as_str()),
            Some(project.as_str()),
            Some(activity.as_str()),
            elapsed.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
        let amber = Status::Attention.color(fg);
        let status: Hsla = if agent.needs_approval {
            amber
        } else if agent.done {
            Status::Done.color(fg)
        } else {
            fg.ink(Ink::Secondary)
        };
        let status_glyph = if agent.needs_approval {
            Icon::new(IconName::CircleAlert)
                .size(IconSize::Xs)
                .color(status)
                .into_any_element()
        } else if agent.done {
            Icon::new(IconName::Check)
                .size(IconSize::Xs)
                .color(status)
                .into_any_element()
        } else {
            terminal_spinner(status, cx).into_any_element()
        };
        let actions = request.map(|_| {
            let (allow_id, deny_id, open_id) =
                (agent.id.clone(), agent.id.clone(), agent.id.clone());
            let button = |kind: &str, label: &'static str, primary: bool| {
                Button::new(
                    SharedString::from(format!("agent-{kind}-{}", agent.id)),
                    label,
                )
                .size(ControlSize::Sm)
                .variant(if primary {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Subtle
                })
            };
            let row = div()
                .mt_1p5()
                .pl(px(DETAIL_INSET))
                .flex()
                .items_center()
                .gap_1p5();
            if question {
                row.child(button("answer", "Answer", true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.select_live_agent(&open_id, cx);
                    },
                )))
            } else {
                // The row's own click opens the thread: a button keeps
                // its press.
                row.child(button("allow", "Allow", true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.answer_permission(&allow_id, true, cx);
                    },
                )))
                .child(button("deny", "Deny", false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.answer_permission(&deny_id, false, cx);
                    },
                )))
            }
        });
        let session_id = agent.id.clone();
        div()
            .id(SharedString::from(format!("live-agent-{}", agent.id)))
            .w_full()
            .flex_none()
            .rounded(px(6.0))
            .px_2()
            .py(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.bg(selection)
                } else if agent.needs_approval {
                    el.bg(amber.opacity(0.08))
                        .hover(move |s| s.bg(amber.opacity(0.14)))
                } else {
                    el.hover(move |s| s.bg(fg.fill(Fill::Selected)))
                }
            })
            .tooltip(Tooltip::text(tip))
            .on_click(cx.listener(move |this, _, _, cx| this.select_live_agent(&session_id, cx)))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(figure_icon(
                        self.project_figure(&agent.cwd),
                        self.project_color(&agent.cwd),
                        live,
                        12.0,
                        cx,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(Text::Md.px())
                            .line_height(px(13.0 * 1.375))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(agent.title.clone()),
                    )
                    .children(elapsed.map(|elapsed| {
                        div()
                            .flex_none()
                            .text_size(Text::Sm.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child(elapsed)
                    })),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(DETAIL_INSET))
                    .text_size(Text::Sm.px())
                    .line_height(px(11.0 * 1.35))
                    .text_color(status)
                    .child(status_glyph)
                    .child(div().min_w_0().flex_1().truncate().child(activity))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(80.0))
                            .truncate()
                            .text_color(fg.ink(Ink::Muted))
                            .child(project),
                    ),
            )
            .children(actions)
            .into_any_element()
    }
}

/// How long the turn has run, or took once done.
pub(crate) fn live_agent_elapsed(agent: &LiveAgent, now: i64) -> Option<String> {
    if agent.done {
        agent.duration_ms.map(format_live_elapsed)
    } else {
        agent
            .started_at
            .map(|started| format_live_elapsed(now - started))
    }
}
