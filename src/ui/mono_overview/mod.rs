//! MonoCode `MonoOverview` in `MonoSidebar`: the Mono's work at a glance,
//! at the right of its chat. A 40px header of tabs (Board, Documents,
//! Habits, Changes: the open one names itself and its count, the others
//! are their icons), the Activity button apart from them since it follows
//! the chat, and Hide overview; its left edge resizes it (340 to 960px,
//! kept in `settings.json`).
//!
//! Documents lists the artifacts it wrote, oldest first; one opens over the
//! list (`ui/artifacts.rs`). Habits is `habits.rs`. Board (`board.rs`,
//! where MonoCode lists Sessions) holds its tasks by column and the other
//! sessions it started. Changes is
//! `changes.rs`; Activity is the transcript's own work rows
//! (`ui/transcript/mono_activity.rs`).

mod board;
mod changes;
mod habits;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::mono_overview::OverviewTab;
use crate::monos::{Mono, MonoState};
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// `border-stroke`.
const STROKE: f32 = 0.07;

/// MonoCode `TABS`, in its order, with Board for Sessions.
const TABS: [(OverviewTab, &str); 4] = [
    (OverviewTab::Board, "Board"),
    (OverviewTab::Documents, "Documents"),
    (OverviewTab::Habits, "Habits"),
    (OverviewTab::Changes, "Changes"),
];

fn tab_icon(tab: OverviewTab, color: Hsla) -> Icon {
    let icon = match tab {
        OverviewTab::Board => Icon::new(IconName::LayoutDashboard),
        OverviewTab::Documents => Icon::new(IconName::File),
        OverviewTab::Habits => Icon::new(IconName::Clock),
        OverviewTab::Changes => ExtraIcon::FileDiff.icon(),
        OverviewTab::Activity => Icon::new(IconName::List),
    };
    icon.size(IconSize::Sm).color(color)
}

/// MonoCode `EmptyState`: its icon over a line on how the tab fills,
/// centred in the room the list would take.
pub(super) fn empty_state(tab: OverviewTab, text: String, fg: Hsla) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h(px(200.0))
        .items_center()
        .justify_center()
        .gap_3()
        .px_6()
        .py_10()
        .child(tab_icon(tab, fg.ink(Ink::Secondary)))
        .child(
            div()
                .max_w(px(288.0))
                .text_center()
                .text_size(Text::Base.px())
                .line_height(px(18.0))
                .text_color(fg.ink(Ink::Secondary))
                .child(text),
        )
        .into_any_element()
}

impl BenCodeApp {
    /// MonoCode `MonoSidebar kind="details"`: `relative flex h-full shrink-0
    /// flex-col border-l border-stroke`, with its resize edge.
    pub(crate) fn render_mono_overview(
        &self,
        mono: &Mono,
        state: &MonoState,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let overview = &self.monos.overview;
        let resizing = overview.resizing.is_some();
        let edge = div()
            .id("mono-overview-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-4.0))
            .w(px(8.0))
            .cursor_col_resize()
            .map(|el| {
                if resizing {
                    el.bg(fg.fill(Fill::Bold))
                } else {
                    el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                }
            })
            .tooltip(Tooltip::text("Resize Mono details"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.monos.overview.width = crate::app::mono_overview::DEFAULT_WIDTH;
                        this.save_settings(cx);
                    } else {
                        this.monos.overview.resizing = Some((
                            crate::ui::scale::logical(event.position.x),
                            this.monos.overview.width,
                        ));
                    }
                    cx.notify();
                }),
            );
        let habit_page = (overview.tab == OverviewTab::Habits)
            .then(|| self.render_habit_route(mono, cx))
            .flatten();
        let paged = overview.document.is_some() || habit_page.is_some();
        let body = if overview.document.is_some() {
            self.render_artifact_reader(cx)
        } else if let Some(page) = habit_page {
            page
        } else if overview.tab == OverviewTab::Changes {
            self.render_mono_changes(mono, cx)
        } else {
            let list = match overview.tab {
                OverviewTab::Board => self.render_mono_board(mono, cx),
                OverviewTab::Documents => self.render_mono_documents(mono, cx),
                OverviewTab::Habits => self.render_mono_habits(mono, cx),
                _ => self.render_mono_activity_tab(mono, state, cx),
            };
            let scroll = div()
                .id("mono-overview-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .py_2()
                .child(list);
            crate::ui::scrollbar::Scrolled::new("mono-overview-scrollbar", scroll)
                .into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(overview.width))
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(fg.opacity(STROKE))
            .child(edge)
            .when(!paged, |el| el.child(self.render_mono_overview_header(cx)))
            .child(body)
            .into_any_element()
    }

    /// The counts on the tabs (MonoCode `counts`): `None` hides it.
    fn mono_overview_count(&self, tab: OverviewTab) -> Option<usize> {
        let mono = self.mono_in_view()?;
        let overview = &self.monos.overview;
        let count = match tab {
            // Its open tasks.
            OverviewTab::Board => overview.board.as_ref()?.iter().filter(|t| t.done_at.is_none()).count(),
            OverviewTab::Documents => self.mono_documents(mono).len(),
            OverviewTab::Habits => overview.habits.as_ref()?.len(),
            OverviewTab::Changes => self.mono_changes_count(),
            OverviewTab::Activity => return None,
        };
        (count > 0).then_some(count)
    }

    /// MonoCode `MonoSidebarHeader` with `OverviewTabs`, the Activity button
    /// and Hide overview.
    fn render_mono_overview_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let current = self.monos.overview.tab;
        let tabs = TABS.iter().map(|&(tab, label)| {
            let count = self.mono_overview_count(tab);
            let dot = tab == OverviewTab::Changes && tab != current && count.is_some();
            self.render_tab_button(tab, label, tab == current, count, dot, cx)
                .on_click(cx.listener(move |this, _, _, cx| this.select_mono_overview_tab(tab, cx)))
                .into_any_element()
        });
        let activity = current == OverviewTab::Activity;
        div()
            .flex()
            .flex_none()
            .items_stretch()
            .h(px(40.0))
            .border_b_1()
            .border_color(fg.opacity(STROKE))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .pl_4()
                    .child(div().flex().items_center().gap_0p5().ml(px(-8.0)).children(tabs)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .px_3()
                    .children((current == OverviewTab::Habits).then(|| {
                        let full = self
                            .monos
                            .overview
                            .habits
                            .as_ref()
                            .is_none_or(|list| list.len() >= crate::monos::habits::HABITS_MAX);
                        IconButton::new("mono-overview-new-habit", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(if full && self.monos.overview.habits.is_some() {
                                format!("At most {} habits", crate::monos::habits::HABITS_MAX)
                            } else {
                                "New habit".to_string()
                            })
                            .disabled(full)
                            .on_click(cx.listener(|this, _, window, cx| this.open_new_habit(window, cx)))
                    }))
                    .child(
                        self.render_tab_button(
                            OverviewTab::Activity,
                            "Activity",
                            activity,
                            None,
                            false,
                            cx,
                        )
                        .tooltip(Tooltip::text(if activity {
                            "Hide activity"
                        } else {
                            "Show activity"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mono_activity(cx))),
                    )
                    .child(
                        IconButton::new("mono-overview-hide", IconName::PanelRight)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Hide overview")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mono_overview(cx))),
                    ),
            )
    }

    /// MonoCode `TabButton`: `h-6 rounded-md px-2 text-[12px]`, the icon
    /// alone until selected, then its name and its count on the accent.
    fn render_tab_button(
        &self,
        tab: OverviewTab,
        label: &'static str,
        selected: bool,
        count: Option<usize>,
        dot: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let accent = colors.accent;
        let ink = if selected { fg } else { fg.ink(Ink::Muted) };
        let group = SharedString::from(format!("mono-overview-tab-{label}"));
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .h(px(24.0))
            .px_2()
            .rounded(px(6.0))
            .text_size(Text::Base.px())
            .cursor_pointer()
            .text_color(ink)
            .map(|el| {
                if selected {
                    el.bg(fg.fill(Fill::Selected))
                } else {
                    el.hover(move |s| s.bg(fg.fill(Fill::Hover)).text_color(fg.ink(Ink::Strong)))
                }
            })
            // Activity's caller names it by what a click does.
            .when(!selected && tab != OverviewTab::Activity, |el| {
                el.tooltip(Tooltip::text(label))
            })
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        tab_icon(tab, ink).group_hover_color(group.clone(), fg.ink(Ink::Strong)),
                    )
                    .when(dot, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .left(px(10.0))
                                .size(px(6.0))
                                .rounded_full()
                                .bg(accent),
                        )
                    }),
            )
            .when(selected, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pl_1p5()
                        .child(label)
                        .children(count.map(|count| {
                            div()
                                .min_w(px(16.0))
                                .rounded(px(4.0))
                                .bg(accent)
                                .px_1()
                                .text_center()
                                .text_size(Text::Xs.px())
                                .line_height(px(16.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(gpui::white())
                                .child(count.to_string())
                        })),
                )
            })
    }

    /// MonoCode `monoSpawnedSessions` over the Mono's chat.
    pub(super) fn mono_launches(&self, mono: &Mono) -> Vec<crate::app::mono_sessions::SpawnedSession> {
        mono.session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|session| crate::app::mono_sessions::chat_launches(&session.blocks))
            .unwrap_or_default()
    }

    /// MonoCode `documents`: the artifacts the Mono's chat holds cards for,
    /// oldest first.
    fn mono_documents(&self, mono: &Mono) -> Vec<crate::app_cli::artifacts::ArtifactCard> {
        mono.session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|session| crate::app_cli::artifacts::chat_cards(&session.blocks))
            .unwrap_or_default()
    }

    /// Documents: what the Mono wrote (MonoCode `DocumentRow`, laid out
    /// like a habit); one opens in the reader.
    fn render_mono_documents(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let documents = self.mono_documents(mono);
        if documents.is_empty() {
            return empty_state(
                OverviewTab::Documents,
                format!(
                    "Ask {} to write something up, like a plan or a summary, and it shows up here.",
                    mono.display_name()
                ),
                fg,
            );
        }
        div()
            .children(documents.into_iter().map(|card| {
                let id = card.id.clone();
                let line = card
                    .summary
                    .clone()
                    .unwrap_or_else(|| card.kind.label().to_string());
                crate::ui::rows::ListRow::new(format!("mono-document-{id}"), card.title.clone())
                    .tile(
                        crate::ui::artifacts::artifact_icon(card.kind)
                            .size(IconSize::Sm)
                            .color(fg.ink(Ink::Secondary)),
                    )
                    .line(line)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_artifact(&id, cx)))
            }))
            .into_any_element()
    }

    /// Activity: the picked turn's steps, or how to pick one.
    fn render_mono_activity_tab(
        &self,
        mono: &Mono,
        _state: &MonoState,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let picked = self
            .monos
            .overview
            .activity
            .as_ref()
            .filter(|picked| mono.session_id.as_deref() == Some(picked.session_id.as_str()));
        picked
            .and_then(|picked| self.render_mono_activity(&picked.session_id, &picked.turn_id, cx))
            .unwrap_or_else(|| {
                empty_state(
                    OverviewTab::Activity,
                    format!(
                        "Select the activity on one of {}'s messages to see each step it took.",
                        mono.display_name()
                    ),
                    fg,
                )
            })
    }
}
