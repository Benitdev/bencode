//! MonoCode `SubagentStack`, `SubagentRow`, `SubagentPanel` and
//! `SubagentMascot` (`AgentTranscript.tsx`): a delegated run as a mascot,
//! its name and how far it got; opened, the trail it left, drawn with the
//! same rows as the main agent's work, then what it reported.

use std::sync::Arc;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*,
};

use super::activity::Trail;
use super::blocks::markdown;
use super::markdown::Tone;
use super::turns::{self, ToolState};
use crate::app::BenCodeApp;
use crate::app::subagents;
use crate::db::Block;
use crate::harness::StepKind;
use crate::ui::mascot::project_mascot;
use crate::ui::rail::widgets::mascot_icon;
use crate::ui::scale::px;
use crate::ui::spinner;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::transcript::selection::OwnsPress;

impl BenCodeApp {
    /// The runs a live turn keeps in place, one row each.
    pub(super) fn render_subagent_stack(
        &self,
        trail: Trail<'_>,
        group: &[usize],
        live: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .min_w_0()
            .children(
                group
                    .iter()
                    .map(|&ix| self.render_subagent_row(trail, ix, live, cx)),
            )
            .into_any_element()
    }

    /// One run. A run that died opens itself, so the reason is not buried
    /// behind a face like every other finished one; a click takes over
    /// from there.
    pub(super) fn render_subagent_row(
        &self,
        trail: Trail<'_>,
        ix: usize,
        live: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let block = &trail.blocks[ix];
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let state = turns::tool_state(block);
        let failed = state == ToolState::Failed;
        let active = live && state == ToolState::Pending;
        let name = subagents::name(block);
        let model = subagents::model(block)
            .map(|id| crate::harness::catalog::native_label(trail.harness, id));
        let steps = subagents::steps(block);
        let status = subagents::status_line(failed, &steps);
        // What it handed back, unless that is the last thing its trail says.
        let report = (state != ToolState::Pending)
            .then(|| turns::tool_detail(block))
            .flatten()
            .filter(|report| {
                steps
                    .iter()
                    .rev()
                    .find(|s| s.kind == StepKind::Message)
                    .is_none_or(|last| last.text.trim() != report.trim())
            })
            .map(str::to_string);
        let key = block.id.clone();
        let open = self
            .transcript_ui
            .subagent_open
            .get(&key)
            .copied()
            .unwrap_or(failed);
        let expandable = !steps.is_empty() || report.is_some();
        let open = open && expandable;

        let ink = if failed {
            colors.danger
        } else if state == ToolState::Pending {
            fg.ink(Ink::Body)
        } else {
            fg.ink(Ink::Muted)
        };
        // The slot keeps the rows' 14px icon width; the sprite sits a size
        // smaller so it weighs the same as the line icons around it.
        let mascot = div()
            .flex()
            .flex_none()
            .size(px(14.0))
            .items_center()
            .justify_center()
            .child(mascot_icon(project_mascot(&name), ink, active, 12.0, cx));
        let group = SharedString::from(format!("subagent-{key}"));
        let title = if active {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(Text::Lg.px())
                .child(spinner::shimmer(name, cx))
        } else {
            let name_color = if failed {
                colors.danger
            } else {
                fg.opacity(0.75)
            };
            let hover = if failed { colors.danger } else { fg };
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(Text::Lg.px())
                .text_color(name_color)
                .group_hover(group.clone(), move |s| s.text_color(hover))
                .child(name)
        };
        let meta = (model.is_some() || !status.is_empty()).then(|| {
            div()
                .flex()
                .flex_none()
                .max_w(gpui::relative(0.55))
                .min_w_0()
                .items_center()
                .gap_2()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Muted))
                .when_some(model, |el, model| {
                    el.child(div().min_w_0().truncate().child(model))
                })
                .when(!status.is_empty(), |el| {
                    el.child(div().flex_none().child(status))
                })
        });
        let chevron = Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .size(IconSize::Xs)
        .color(fg.ink(Ink::Faint));
        let hover_bg = fg.fill(Fill::Selected);
        let header = div()
            .id(SharedString::from(format!("subagent-row-{key}")))
            .group(group)
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .mx(px(-6.0))
            .px(px(6.0))
            .py_1()
            .rounded(px(6.0))
            .when(open, |el| el.bg(hover_bg))
            .child(mascot)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    // Centred like the mascot and chevron: on the name's
                    // baseline the smaller line sat low beside them.
                    .items_center()
                    .gap_2()
                    .child(title)
                    .children(meta),
            )
            // Held room, so the chevron arriving moves nothing.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .size(px(14.0))
                    .items_center()
                    .justify_center()
                    .when(expandable, |el| el.child(chevron)),
            )
            .when(expandable, |el| {
                let key = key.clone();
                el.cursor_pointer()
                    .owns_press()
                    .hover(move |s| s.bg(hover_bg))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.transcript_ui.subagent_open.insert(key.clone(), !open);
                        this.transcript_ui.reflow += 1;
                        this.redraw_regions(
                            &[
                                crate::ui::region::Region::Chat,
                                crate::ui::region::Region::Mono,
                            ],
                            cx,
                        );
                    }))
            });
        if !open {
            return header.into_any_element();
        }

        // The run's trail as transcript blocks, grouped the way the main
        // agent's own work is.
        let step_blocks: Vec<Arc<Block>> = steps
            .iter()
            .map(|step| Arc::new(subagents::step_block(step)))
            .collect();
        let step_trail = Trail {
            blocks: &step_blocks,
            select_as: Some(trail.select_as.unwrap_or(ix)),
            ..trail
        };
        let all: Vec<usize> = (0..step_blocks.len()).collect();
        let mono = cx.theme().mono_family.clone();
        let danger = colors.danger;
        div()
            .min_w_0()
            .child(header)
            .child(
                div()
                    .min_w_0()
                    .pb_1()
                    .when(!all.is_empty(), |el| {
                        el.child(self.render_phase_list(step_trail, &all, !active, cx))
                    })
                    .when_some(report, |el, report| {
                        let body = if failed {
                            div()
                                .font_family(mono)
                                .text_size(Text::Base.px())
                                .line_height(px(20.0))
                                .text_color(danger.opacity(0.8))
                                .child(report)
                                .into_any_element()
                        } else {
                            markdown(
                                SharedString::from(format!("subagent-report-{key}")),
                                &report,
                                None,
                                Tone::Fold,
                                self.seg_ctx(
                                    trail.session_id,
                                    step_trail.select_as.unwrap_or(ix),
                                    false,
                                ),
                                cx,
                            )
                        };
                        el.child(div().py_1().child(body))
                    }),
            )
            .into_any_element()
    }
}
