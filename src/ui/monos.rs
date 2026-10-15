//! A Mono in the main area (MonoCode `App.tsx`'s `monoCovers`,
//! `MonoHeader`, `MonoDetails`, `MonoProjects`, `ConfirmReset`): its
//! conversation in place of the sidebar, the tabs and the footer, under its
//! title bar (`ui/titlebar/mono.rs`), with its details beside the chat, and
//! the greeting that fills the chat before the first message.
//!
//! Details holds its look, its projects and its soul (`SOUL.md`); the
//! title bar's Overview and gear both show it until MonoCode's Overview
//! panel and settings dialog are ported. The model is the composer's
//! (`ui/composer/mono.rs`). Habits, memory and the sessions and documents
//! a Mono starts are not ported.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::menus::{Menu, MenuItem, OverflowMenu};
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::session_folders::{FOLDER_COLORS, palette_color};
use crate::monos::{Mono, MonoState, MonoStatus, projects_phrase};
use crate::ui::app_callback::app_callback;
use crate::ui::mascot::{MASCOT_NAMES, MascotMotion, mascot_for, moving_sprite, pixel_sprite};
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;

/// MonoCode `MonoSidebar`: `MIN_WIDTH`.
const DETAILS_WIDTH: f32 = 340.0;
/// MonoCode `border-stroke`.
const STROKE_OPACITY: f32 = 0.07;

impl BenCodeApp {
    /// MonoCode `monoCovers`: a Mono's conversation is what the main area
    /// shows (a view opened over it still wins).
    pub fn mono_covers(&self) -> bool {
        self.surface.is_none() && self.monos.view.is_some()
    }

    /// The Mono's column: its title bar, then the chat and details. It
    /// covers the footer, as MonoCode's does.
    pub(crate) fn render_mono_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.mono_covers() {
            return None;
        }
        let mono = self.mono_in_view()?.clone();
        let state = self.mono_state(&mono);
        // The plan's usage shows under its composer: read once, as the
        // footer's chip does.
        let harness = self.selected_session().map(|s| s.harness.clone());
        if let Some(target) = self.usage_target().filter(|target| {
            target.available
                && harness
                    .as_deref()
                    .is_some_and(crate::ui::composer::mono::shows_usage)
        }) {
            self.load_rate_limits(target.provider, &target.account_id, false, cx);
        }
        let glass = self.glass(cx);
        let bg = cx.theme().colors.bg;
        let chat = match self.mono_view_session().map(str::to_string) {
            Some(session_id) => self.render_session_pane(&session_id, false, cx),
            None => EmptyState::new(
                "mono-loading",
                IconName::MessageSquare,
                "Opening the conversation…",
            )
            .into_any_element(),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(glass.body(bg))
                .child(self.render_mono_titlebar(&mono, &state, cx))
                // The plan's usage is under the composer instead.
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex_1().min_w_0().h_full().child(chat))
                        .when(self.mono_details_shown(), |el| {
                            el.child(self.render_mono_details(&mono, &state, cx))
                        }),
                )
                .into_any_element(),
        )
    }

    /// MonoCode `MonoDetails`: `mono-details-panel flex h-full shrink-0
    /// flex-col border-l border-stroke`, under `MonoSidebarHeader`. Its
    /// front page (`MonoSettingsPage`): the face and name, the mascot and
    /// colour pickers, then its projects and its soul.
    fn render_mono_details(
        &self,
        mono: &Mono,
        state: &MonoState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let stroke = fg.opacity(STROKE_OPACITY);
        let color = self.mono_color(mono);
        // `flex h-10 shrink-0 items-stretch border-b border-stroke`
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(crate::ui::sidebar::TITLEBAR_HEIGHT))
            .pl_4()
            .pr_3()
            .border_b_1()
            .border_color(stroke)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child("Details"),
            )
            .child(
                IconButton::new("mono-details-hide", IconName::PanelRight)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Hide details")
                    .on_click(cx.listener(|this, _, _, cx| this.close_mono_details(cx))),
            );
        // `flex flex-col items-center px-3 pb-4 pt-6`
        let profile = div()
            .flex()
            .flex_col()
            .items_center()
            .px_3()
            .pb_4()
            .pt_6()
            .child(pixel_sprite(
                &self.mono_mascot(mono).rest,
                px(56.0),
                color,
                false,
            ))
            .child(
                // `mt-3 h-8 text-center text-[15px] font-medium`
                div()
                    .mt_3()
                    .max_w_full()
                    .truncate()
                    .text_size(px(15.0))
                    .line_height(px(24.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(mono.display_name()),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .max_w_full()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.45))
                    .child(mono_status_line(state, color, cx)),
            );
        let body = div()
            .id("mono-details-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(profile)
            .child(
                // `flex flex-col gap-3 px-3 pb-4`
                div()
                    .px_3()
                    .pb_4()
                    .child(self.render_mono_mascot_picker(mono, cx))
                    .child(div().mt_3().child(self.render_mono_color_picker(mono, cx))),
            )
            .child(self.render_mono_projects(mono, cx))
            .child(
                details_section("Soul", None, stroke, fg)
                    .child(
                        // MonoCode's Soul row: what the page is for.
                        div()
                            .px_2()
                            .pb_2()
                            .text_size(px(12.0))
                            .line_height(px(18.0))
                            .text_color(fg.opacity(0.4))
                            .child(
                                "Who it is and the rules it follows. Always part of what \
                                 it knows.",
                            ),
                    )
                    .child(
                        div()
                            .mx_2()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(fg.opacity(0.08))
                            .bg(fg.opacity(0.03))
                            .px(px(10.0))
                            .py(px(8.0))
                            .font_family(cx.theme().mono_family.clone())
                            .text_size(px(12.0))
                            .line_height(px(18.0))
                            .child(self.monos.soul_input.clone()),
                    )
                    .children(self.monos.soul_error.clone().map(|err| {
                        div()
                            .px_2()
                            .pt_2()
                            .text_size(px(12.0))
                            .text_color(colors.danger)
                            .child(err)
                    })),
            );
        // MonoCode's Settings row at the panel's foot (`mt-auto p-2`): here
        // the one thing its page holds that BenCode has, the chat's reset.
        let reset = div()
            .flex_none()
            .p_2()
            .border_t_1()
            .border_color(stroke)
            .child(
                div()
                    .id("mono-reset")
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(8.0))
                    .px_3()
                    .py_2()
                    .hover(move |s| s.bg(fg.opacity(0.05)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.monos.pending_reset = true;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(20.0))
                                    .text_color(fg.opacity(0.9))
                                    .child("Reset conversation"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .line_height(px(20.0))
                                    .text_color(fg.opacity(0.4))
                                    .child("Starts its chat over; the rest stays."),
                            ),
                    )
                    .child(
                        Icon::new(IconName::RotateCcw)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.3)),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .border_l_1()
            .border_color(stroke)
            .child(header)
            .child(Scrolled::new("mono-details-scrollbar", body))
            .child(reset)
    }

    /// MonoCode `MascotPicker`: `grid grid-cols-10 gap-0.5 px-1`, each
    /// `h-8 rounded-md` with the mascot in the Mono's colour at `size-5`.
    fn render_mono_mascot_picker(&self, mono: &Mono, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let color = self.mono_color(mono);
        let selected_bg = fg.opacity(if theme.is_dark() { 0.1 } else { 0.06 });
        div()
            .flex()
            .gap(px(2.0))
            .px_1()
            .children(MASCOT_NAMES.iter().map(|name| {
                let selected = *name == mono.mascot;
                let id = mono.id.clone();
                div()
                    .id(SharedString::from(format!("mono-mascot-{name}")))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(32.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .map(|el| {
                        if selected {
                            el.bg(selected_bg).border_1().border_color(fg.opacity(0.2))
                        } else {
                            el.hover(move |s| s.bg(fg.opacity(0.06)))
                        }
                    })
                    .tooltip(Tooltip::text(*name))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_mono_mascot(&id, name, cx)),
                    )
                    .child(pixel_sprite(
                        &mascot_for(name, Some(name)).rest,
                        px(20.0),
                        color,
                        false,
                    ))
            }))
    }

    /// MonoCode `ColorPicker`: the presets in the mascots' columns, the
    /// picked one ringed. A custom colour is set from the rail's menu.
    fn render_mono_color_picker(&self, mono: &Mono, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .gap(px(2.0))
            .px_1()
            .children((1..FOLDER_COLORS.len()).map(|index| {
                let selected = mono.custom_color.is_none() && mono.color_index == index;
                let id = mono.id.clone();
                div()
                    .id(("mono-color", index))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(24.0))
                    .items_center()
                    .justify_center()
                    .tooltip(Tooltip::text(format!("Color {index}")))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_mono_color(&id, index, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .size(px(18.0))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .when(selected, |el| el.border_2().border_color(fg.opacity(0.8)))
                            .child(
                                div()
                                    .size(px(if selected { 10.0 } else { 14.0 }))
                                    .rounded_full()
                                    .bg(palette_color(index).unwrap_or_default()),
                            ),
                    )
            }))
    }

    /// MonoCode `MonoProjects`: each project as the rail shows it (its
    /// mascot in its colour, then its name) with a remove button on hover;
    /// the section's + adds one of the rail's others, or any folder.
    fn render_mono_projects(&self, mono: &Mono, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let stroke = fg.opacity(STROKE_OPACITY);
        let projects = self.mono_projects(mono);
        let item = |path: String| {
            let (id, target) = (mono.id.clone(), path.clone());
            MenuItem::new(self.rail_project_label(&path))
                .icon(IconName::Folder)
                .on_click(app_callback(cx, move |this, cx| {
                    this.add_mono_project(&id, &target, cx)
                }))
        };
        let choices = self.mono_project_choices(mono);
        let has_choices = !choices.is_empty();
        let mut menu = choices
            .into_iter()
            .fold(Menu::new(), |menu, path| menu.item(item(path)));
        if has_choices {
            menu = menu.separator();
        }
        let pick_id = mono.id.clone();
        let menu = menu.item(
            MenuItem::new("Choose folder…")
                .icon(IconName::FolderPlus)
                .on_click(app_callback(cx, move |this, cx| {
                    this.pick_mono_project(&pick_id, cx)
                })),
        );
        let add = OverflowMenu::new("mono-add-project", menu)
            .icon(IconName::Plus)
            .tooltip("Add project");
        details_section("Projects", Some(add.into_any_element()), stroke, fg)
            .when(projects.is_empty(), |el| {
                el.child(
                    div()
                        .px_2()
                        .text_size(px(12.0))
                        .line_height(px(18.0))
                        .text_color(fg.opacity(0.4))
                        .child("None yet. It helps with whatever you bring."),
                )
            })
            .children(projects.into_iter().enumerate().map(|(ix, (name, path))| {
                let group = SharedString::from(format!("mono-project-{ix}"));
                let remove_group = SharedString::from(format!("mono-project-remove-{ix}"));
                let (id, target) = (mono.id.clone(), path.clone());
                // `group flex h-7 min-w-0 items-center gap-2 text-content/85`
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(fg.opacity(0.05)))
                    .tooltip(Tooltip::text(path.clone()))
                    .child(pixel_sprite(
                        &self.project_mascot(&path).rest,
                        px(12.0),
                        self.project_color(&path),
                        false,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.85))
                            .child(name),
                    )
                    .child(
                        // `grid size-5 rounded text-content/45 opacity-0
                        // hover:bg-content/8 group-hover:opacity-100`
                        div()
                            .id(("mono-project-remove", ix))
                            .group(remove_group.clone())
                            .flex()
                            .flex_none()
                            .size_5()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .hover(move |s| s.bg(fg.opacity(0.08)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_mono_project(&id, &target, cx)
                            }))
                            .child(
                                Icon::new(IconName::X)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.45))
                                    .group_hover_color(remove_group, fg),
                            ),
                    )
            }))
    }

    /// The Mono's delete and reset confirmations. Full-window dialogs: in
    /// `BenCodeApp::render`'s `dialogs` list.
    pub(crate) fn render_mono_dialogs(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if let Some(id) = self.monos.pending_delete.clone() {
            let name = self.mono(&id)?.display_name();
            let close = app_callback(cx, |this, cx| {
                this.monos.pending_delete = None;
                cx.notify();
            });
            let delete = app_callback(cx, move |this, cx| this.delete_mono(&id, cx));
            return Some(
                ConfirmDialog::new(
                    "mono-delete-confirm",
                    format!("Delete {name}?"),
                    "Its conversation is deleted with it. Its standing instructions stay on \
                     disk.",
                    close,
                )
                .confirm("Delete")
                .destructive()
                .on_confirm(delete)
                .into_any_element(),
            );
        }
        if !self.monos.pending_reset {
            return None;
        }
        let name = self.mono_in_view()?.display_name();
        let close = app_callback(cx, |this, cx| {
            this.monos.pending_reset = false;
            cx.notify();
        });
        let reset = app_callback(cx, |this, cx| this.reset_mono_session(cx));
        Some(
            ConfirmDialog::new(
                "mono-reset-confirm",
                format!("Reset {name}'s conversation?"),
                "The chat starts over. Its name, look, projects and standing instructions \
                 stay.",
                close,
            )
            .confirm("Reset")
            .destructive()
            .on_confirm(reset)
            .into_any_element(),
        )
    }

    /// MonoCode `MonoHeader` with `greeting`: before the first message the
    /// header fills the chat (`min-h-full justify-center py-12`), the
    /// mascot at `size-14` standing on its name pill (`-mt-2`, drawn over
    /// it) and saying hello.
    pub(crate) fn render_mono_greeting(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let names: Vec<String> = self
            .mono_projects(mono)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        // `.mono-pill`: the text colour at 14% and 6%; white in light mode,
        // lifted by a shadow.
        let pill = div()
            .mt(px(-8.0))
            .flex()
            .h(px(32.0))
            .items_center()
            .rounded_full()
            .border_1()
            .px(px(14.0))
            .map(|el| {
                if theme.is_dark() {
                    el.border_color(fg.opacity(0.14)).bg(fg.opacity(0.06))
                } else {
                    el.border_color(gpui::black().opacity(0.06))
                        .bg(gpui::white())
                        .shadow_md()
                }
            })
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(mono.display_name());
        div()
            .id("mono-greeting")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .min_h_full()
                    .px_6()
                    .py_12()
                    .text_center()
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(div().h(px(56.0)))
                            .child(pill)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .flex()
                                    .justify_center()
                                    .child(moving_sprite(
                                        self.mono_mascot(mono),
                                        56.0,
                                        self.mono_color(mono),
                                        MascotMotion::Bob,
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        // `mt-3 max-w-sm text-[13px] leading-relaxed text-content/50`
                        div()
                            .mt_3()
                            .max_w(px(384.0))
                            .text_size(px(13.0))
                            .line_height(px(21.0))
                            .text_color(fg.opacity(0.5))
                            .child(greeting_text(&names)),
                    ),
            )
            .into_any_element()
    }
}

/// MonoCode `MonoHeader`'s hello, with `monoProjectsPhrase`.
fn greeting_text(projects: &[String]) -> String {
    if projects.is_empty() {
        "Add the projects I should work on in my settings, or just ask me something.".to_string()
    } else {
        format!(
            "I can see every session, note and folder in {}. Ask what’s going on, or hand me \
             work.",
            projects_phrase(projects)
        )
    }
}

/// MonoCode `Section`: `border-t border-stroke px-2 pb-4 pt-3`, its title
/// `text-[11px] font-medium uppercase tracking-wide text-content/40` with
/// an action at the row's end.
fn details_section(
    title: &'static str,
    action: Option<AnyElement>,
    stroke: Hsla,
    fg: Hsla,
) -> gpui::Div {
    div()
        .border_t_1()
        .border_color(stroke)
        .px_2()
        .pb_4()
        .pt_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(24.0))
                .px_2()
                .mb_1()
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg.opacity(0.4))
                        .child(title.to_uppercase()),
                )
                .children(action),
        )
}

/// MonoCode `MonoStatus`: a dot (accent when it needs you, its colour while
/// it works, grey at rest), the state, and what it is on.
fn mono_status_line(state: &MonoState, color: Hsla, cx: &Context<BenCodeApp>) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let dot = match state.status {
        MonoStatus::NeedsYou => colors.accent,
        MonoStatus::Working => color,
        MonoStatus::Idle => colors.fg.opacity(0.3),
    };
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .child(div().flex_none().size(px(8.0)).rounded_full().bg(dot))
        .child(div().flex_none().child(state.status.label()))
        .children(state.activity.clone().map(|activity| {
            div()
                .flex()
                .min_w_0()
                .gap_1p5()
                .child(div().flex_none().opacity(0.6).child("·"))
                .child(div().min_w_0().truncate().child(activity))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeting_uses_monocodes_words() {
        assert_eq!(
            greeting_text(&[]),
            "Add the projects I should work on in my settings, or just ask me something."
        );
        assert_eq!(
            greeting_text(&["bencode".into(), "site".into()]),
            "I can see every session, note and folder in bencode and site. Ask what’s going \
             on, or hand me work."
        );
    }
}
