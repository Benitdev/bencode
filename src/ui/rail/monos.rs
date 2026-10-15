//! MonoCode `MonoRailSection`: the user's Monos, above the projects. Each
//! opens like a project does, but into its conversation. Its menu is the
//! rail's own (`menu`, `MenuTarget::Mono`).
//!
//! Not ported: the "Meet Monos" intro, unread counts and drag to reorder.

use ely_gpui_component::primitives::{IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, Styled, div, prelude::*, relative,
};

use super::widgets::{ROW_HEIGHT, hover_control, section_button};
use crate::app::BenCodeApp;
use crate::monos::{Mono, MonoState, MonoStatus, projects_phrase};
use crate::ui::scale::px;

impl BenCodeApp {
    /// `mb-2 shrink-0`: the "Monos" header with its New mono button, then
    /// the rows; with none yet, a dashed row is the way to add one.
    pub(super) fn render_rail_monos(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let monos = &self.settings.monos;
        let add = (!monos.is_empty()).then(|| {
            section_button("rail-new-mono", IconName::Plus, "New mono", false, fg)
                .on_click(cx.listener(|this, _, _, cx| this.create_mono(cx)))
                .into_any_element()
        });
        div()
            .flex_none()
            .mb_2()
            .child(self.render_rail_section_header("Monos", add, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .px_2()
                    .when(monos.is_empty(), |el| {
                        // `grid h-8 w-full place-items-center rounded-md
                        // border border-dashed border-content/15 text-content/50`
                        el.child(
                            div()
                                .id("rail-add-mono")
                                .group("rail-add-mono")
                                .flex()
                                .h(px(ROW_HEIGHT))
                                .w_full()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .border_1()
                                .border_dashed()
                                .border_color(fg.opacity(0.15))
                                .hover(move |s| {
                                    s.border_color(fg.opacity(0.3)).bg(fg.opacity(0.05))
                                })
                                .tooltip(Tooltip::text("New mono"))
                                .on_click(cx.listener(|this, _, _, cx| this.create_mono(cx)))
                                .child(
                                    ely_gpui_component::primitives::Icon::new(IconName::Plus)
                                        .size(IconSize::Sm)
                                        .color(fg.opacity(0.5))
                                        .group_hover_color("rail-add-mono", fg),
                                ),
                        )
                    })
                    .children(monos.iter().map(|mono| self.render_mono_card(mono, cx))),
            )
    }

    /// One Mono's row: `group relative flex h-8 items-stretch rounded-md
    /// px-2`, `bg-selection-strong` while open, else `opacity-65`.
    fn render_mono_card(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let id = format!("rail-mono-{}", mono.id);
        let group = SharedString::from(id.clone());
        let name = mono.display_name();
        let state = self.mono_state(mono);
        let working = state.status == MonoStatus::Working;
        let selected = self.surface.is_none() && self.monos.view.as_deref() == Some(&mono.id);
        let strong = fg.opacity(if theme.is_dark() { 0.12 } else { 0.07 });
        let hover = fg.opacity(0.05);
        let label = if working {
            crate::ui::spinner::shimmer(name.clone(), cx)
        } else {
            name.clone().into_any_element()
        };
        let (open_id, menu_id, options_id) = (mono.id.clone(), mono.id.clone(), mono.id.clone());
        div()
            .id(SharedString::from(id.clone()))
            .group(group.clone())
            .relative()
            .flex()
            .items_stretch()
            .h(px(ROW_HEIGHT))
            .px_2()
            .rounded(px(6.0))
            .map(|el| {
                if selected {
                    el.bg(strong)
                } else {
                    el.opacity(0.65).hover(move |s| s.bg(hover))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.open_mono(&open_id, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_rail_mono_menu(&menu_id, event.position, cx);
                }),
            )
            .child(
                // `flex min-w-0 flex-1 items-center gap-2 group-hover:pr-6`
                div()
                    .id(SharedString::from(format!("{id}-main")))
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .group_hover(group.clone(), |s| s.pr(px(24.0)))
                    .tooltip(Tooltip::text(self.mono_card_title(mono, &state)))
                    .child(self.render_mono_rail_mascot(mono, &state, cx))
                    .child(
                        // `min-w-0 flex-1 truncate text-sm font-medium leading-tight`
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(relative(1.25))
                            .child(label),
                    ),
            )
            .child(
                hover_control(
                    SharedString::from(format!("{id}-options")),
                    IconName::Ellipsis,
                    IconSize::Md,
                    "Mono options",
                    &group,
                    fg,
                )
                .right_1()
                .top(px(4.0))
                .size_6()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_rail_mono_menu(&options_id, event.position, cx);
                    }),
                ),
            )
            .into_any_element()
    }

    /// The row's title: its name, its projects, and what it is doing.
    fn mono_card_title(&self, mono: &Mono, state: &MonoState) -> String {
        let names: Vec<String> = self
            .mono_projects(mono)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let projects = if names.is_empty() {
            "No projects yet".to_string()
        } else {
            projects_phrase(&names)
        };
        let status = (state.status != MonoStatus::Idle).then(|| {
            state
                .activity
                .clone()
                .unwrap_or_else(|| state.status.label().to_string())
        });
        [Some(mono.display_name()), Some(projects), status]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// MonoCode `MonoRailMascot`: the mascot in `size-4` (hopping while it
    /// works, its bulb lit while it needs you) with its status circle at the
    /// top right (`size-1.5`, ringed with the background).
    fn render_mono_rail_mascot(
        &self,
        mono: &Mono,
        state: &MonoState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let color = self.mono_color(mono);
        let dot = match state.status {
            MonoStatus::Working => color,
            MonoStatus::NeedsYou => colors.accent,
            MonoStatus::Idle => colors.fg.opacity(0.3),
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .size_4()
            .items_center()
            .justify_center()
            .child(crate::ui::mascot::pixel_mascot(
                self.mono_mascot(mono),
                12.0,
                color,
                crate::ui::mascot::MascotLook::for_status(state.status, &mono.mascot),
                cx,
            ))
            .child(
                div()
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .size(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(colors.bg)
                    .child(div().size(px(6.0)).rounded_full().bg(dot)),
            )
    }
}
