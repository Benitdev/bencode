//! MonoCode `LiveAgentsPreview`: the "Working" card above the rail's
//! Settings row (or at the foot of the sidebar while the rail is closed).
//! It shows once two threads are in flight, lists four until expanded, and
//! a click jumps to the thread in its project. The list itself is
//! `app/live_agents.rs`.

use std::time::Duration;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*,
    rgb,
};

use super::project_name;
use super::widgets::{AMBER_400, mascot_icon};
use crate::app::BenCodeApp;
use crate::app::live_agents::{LiveAgent, format_live_elapsed};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::spinner::terminal_spinner;

/// MonoCode `LIVE_AGENT_MIN` / `LIVE_AGENT_CAP`.
const LIVE_AGENT_MIN: usize = 2;
const LIVE_AGENT_CAP: usize = 4;
/// MonoCode `max-h-[45vh]`: the expanded list's share of the window.
const EXPANDED_HEIGHT: f32 = 0.45;
/// MonoCode `text-emerald-400`.
const EMERALD_400: u32 = 0x34d399;
/// Tailwind `animate-pulse`.
const PULSE: Duration = Duration::from_secs(2);

/// What the card keeps between frames.
#[derive(Default)]
pub struct LiveAgentsUi {
    pub expanded: bool,
    /// The window's height at 100%, for the expanded list's limit.
    pub window_height: f32,
}

impl BenCodeApp {
    /// The card, or nothing with fewer than two agents. `bottom_spacing`
    /// is MonoCode's `pb-2`, for where nothing follows it.
    pub(crate) fn render_live_agents(&self, bottom_spacing: bool, cx: &Context<Self>) -> Option<AnyElement> {
        let agents = self.live_agents();
        if agents.len() < LIVE_AGENT_MIN {
            return None;
        }
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let expanded = self.live_agents_ui.expanded;
        let extra = agents.len().saturating_sub(LIVE_AGENT_CAP);
        let shown = if expanded { agents.len() } else { agents.len() - extra };
        let now = crate::app::now_ms();

        // `flex items-center gap-2 px-3.5 py-1.5`
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px(px(14.0))
            .py(px(6.0))
            .child(pulse_dot(accent, cx.reduce_motion()))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.5))
                    .child("Working"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.4))
                    .child(agents.len().to_string()),
            );
        // `flex flex-col gap-px px-1`, scrolling once expanded.
        let list = div()
            .id("live-agents-list")
            .flex()
            .flex_col()
            .gap(px(1.0))
            .px_1()
            .when(extra == 0, |el| el.pb_1())
            .when(expanded, |el| {
                el.max_h(px(self.live_agents_ui.window_height * EXPANDED_HEIGHT))
                    .overflow_y_scroll()
            })
            .children(agents[..shown].iter().map(|agent| self.render_live_agent(agent, now, cx)));
        // `flex w-full items-center justify-center gap-1 px-2 py-1.5
        // text-[11px] text-content/50 hover:bg-content/8 hover:text-content`
        let more = (extra > 0).then(|| {
            div()
                .id("live-agents-more")
                .group("live-agents-more")
                .flex()
                .w_full()
                .items_center()
                .justify_center()
                .gap_1()
                .px_2()
                .py(px(6.0))
                .text_size(px(11.0))
                .text_color(fg.opacity(0.5))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.opacity(0.08)).text_color(fg))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.live_agents_ui.expanded = !this.live_agents_ui.expanded;
                    cx.notify();
                }))
                .child(
                    Icon::new(if expanded { IconName::ChevronUp } else { IconName::ChevronDown })
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.5))
                        .group_hover_color("live-agents-more", fg),
                )
                .child(if expanded {
                    "Show less".to_string()
                } else {
                    format!("{extra} more")
                })
        });
        Some(
            // `shrink-0 px-2`, around `overflow-hidden rounded-lg bg-content/5`
            div()
                .flex_none()
                .px_2()
                .when(bottom_spacing, |el| el.pb_2())
                .child(
                    div()
                        .overflow_hidden()
                        .rounded(px(8.0))
                        .bg(fg.opacity(0.05))
                        .child(header)
                        .child(list)
                        .children(more),
                )
                .into_any_element(),
        )
    }

    /// MonoCode `LiveAgentCard`: the mascot and title, what the agent is
    /// doing, then its harness, project and elapsed time.
    fn render_live_agent(&self, agent: &LiveAgent, now: i64, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
        let seed = project_name(&agent.cwd);
        let project = self.settings.rail.label(&agent.cwd, seed).to_string();
        let elapsed = if agent.done {
            agent.duration_ms.map(format_live_elapsed)
        } else {
            agent.started_at.map(|started| format_live_elapsed(now - started))
        };
        let activity = if agent.needs_approval {
            "Need approval"
        } else if agent.done {
            "Done"
        } else {
            agent.activity.as_str()
        };
        let live = !agent.needs_approval && !agent.done;
        let selected = self.selected_session_id.as_deref() == Some(agent.id.as_str());
        let tip = [Some(agent.title.as_str()), Some(project.as_str()), Some(activity), elapsed.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let id = SharedString::from(format!("live-agent-{}", agent.id));
        let status: Hsla = if agent.needs_approval {
            rgb(AMBER_400).into()
        } else if agent.done {
            rgb(EMERALD_400).into()
        } else {
            fg.opacity(0.5)
        };
        // `mt-1 flex min-w-0 items-center gap-1.5 pl-4 text-[11px] leading-tight`
        let line = |color: Hsla| {
            div()
                .mt_1()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(6.0))
                .pl_4()
                .text_size(px(11.0))
                .line_height(px(11.0 * 1.25))
                .text_color(color)
        };
        let session_id = agent.id.clone();
        // `flex w-full flex-col rounded-md px-2 py-1.5 text-left`
        div()
            .id(id.clone())
            .flex()
            .w_full()
            .flex_none()
            .flex_col()
            .rounded(px(6.0))
            .px_2()
            .py(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.bg(selection)
                } else {
                    el.hover(move |s| s.bg(fg.opacity(0.08)))
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
                    .child(mascot_icon(
                        self.project_mascot(&agent.cwd),
                        self.project_color(&agent.cwd),
                        live,
                        &id,
                        8.0,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.0))
                            .line_height(px(13.0 * 1.375))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(agent.title.clone()),
                    ),
            )
            .child(
                line(status)
                    .child(if agent.needs_approval {
                        Icon::new(IconName::CircleAlert).size(IconSize::Xs).color(status).into_any_element()
                    } else if agent.done {
                        Icon::new(IconName::Check).size(IconSize::Xs).color(status).into_any_element()
                    } else {
                        terminal_spinner(status, cx).into_any_element()
                    })
                    .child(div().min_w_0().truncate().child(activity.to_string())),
            )
            .child(
                line(fg.opacity(0.45))
                    .child(HarnessIcon::new(agent.harness.clone()).size(px(12.0)))
                    .child(div().min_w_0().flex_1().truncate().child(project))
                    .children(elapsed.map(|elapsed| div().flex_none().child(elapsed))),
            )
            .into_any_element()
    }
}

/// `size-1.5 rounded-full bg-accent motion-safe:animate-pulse`.
fn pulse_dot(accent: Hsla, reduced: bool) -> AnyElement {
    let dot = div().flex_none().size(px(6.0)).rounded_full().bg(accent);
    if reduced {
        return dot.into_any_element();
    }
    dot.with_animation("live-agents-pulse", Animation::new(PULSE).repeat(), |el, delta| {
        // Full, down to half at the midpoint, and back.
        let dip = (delta * std::f32::consts::TAU).cos() * 0.25 + 0.75;
        el.opacity(dip)
    })
    .into_any_element()
}
