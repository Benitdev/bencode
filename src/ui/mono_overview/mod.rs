//! MonoCode `MonoOverview` in `MonoSidebar`: the Mono's work at a glance,
//! at the right of its chat. A 40px header of tabs (Sessions, Documents,
//! Habits, Changes: the open one names itself and its count, the others
//! are their icons), the Activity button apart from them since it follows
//! the chat, and Hide overview; its left edge resizes it (340 to 960px,
//! kept in `settings.json`).
//!
//! BenCode's Monos start no sessions, write no artifacts and run no habits
//! (MonoCode's `app` CLI), so Sessions lists the threads in its projects,
//! Documents the files in its folder, and Habits what `habits.json` holds
//! (empty until habits are ported). Changes is `changes.rs`; Activity is
//! the transcript's own work rows (`ui/transcript/mono_activity.rs`).

mod changes;

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::mono_overview::{OverviewTab, visible_sessions};
use crate::monos::{Mono, MonoState};
use crate::ui::HarnessIcon;
use crate::ui::icons::ExtraIcon;
use crate::ui::scale::px;

/// `border-stroke`.
const STROKE: f32 = 0.07;

/// MonoCode `TABS`, in its order.
const TABS: [(OverviewTab, &str); 4] = [
    (OverviewTab::Sessions, "Sessions"),
    (OverviewTab::Documents, "Documents"),
    (OverviewTab::Habits, "Habits"),
    (OverviewTab::Changes, "Changes"),
];

fn tab_icon(tab: OverviewTab, color: Hsla) -> Icon {
    let icon = match tab {
        OverviewTab::Sessions => Icon::new(IconName::MessageSquare),
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
        .child(tab_icon(tab, fg.opacity(0.55)))
        .child(
            div()
                .max_w(px(288.0))
                .text_center()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(fg.opacity(0.55))
                .child(text),
        )
        .into_any_element()
}

/// A row laid out like MonoCode's habit and document rows: `flex gap-2.5
/// rounded-md px-2 py-2 hover:bg-content/5`, a `size-7` tile, a title in
/// 12px and a line in 11px.
pub(super) fn list_row(
    id: SharedString,
    tile: AnyElement,
    title: String,
    line: String,
    fg: Hsla,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(10.0))
        .rounded(px(6.0))
        .px_2()
        .py_2()
        .cursor_pointer()
        .hover(move |s| s.bg(fg.opacity(0.05)))
        .child(
            div()
                .flex()
                .flex_none()
                .size(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .bg(fg.opacity(0.06))
                .child(tile),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.0))
                        .line_height(px(20.0))
                        .text_color(fg.opacity(0.85))
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .text_color(fg.opacity(0.4))
                        .child(line),
                ),
        )
}

/// "4.2 KB", "812 B".
fn size_label(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
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
                    el.bg(fg.opacity(0.15))
                } else {
                    el.hover(move |s| s.bg(fg.opacity(0.1)))
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
        let body = if let Some(document) = &overview.document {
            self.render_mono_document(document, cx)
        } else if overview.tab == OverviewTab::Changes {
            self.render_mono_changes(mono, cx)
        } else {
            let list = match overview.tab {
                OverviewTab::Sessions => self.render_mono_sessions(mono, cx),
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
            .when(overview.document.is_none(), |el| {
                el.child(self.render_mono_overview_header(cx))
            })
            .child(body)
            .into_any_element()
    }

    /// The counts on the tabs (MonoCode `counts`): `None` hides it.
    fn mono_overview_count(&self, tab: OverviewTab) -> Option<usize> {
        let mono = self.mono_in_view()?;
        let overview = &self.monos.overview;
        let count = match tab {
            OverviewTab::Sessions => visible_sessions(&self.sessions, &mono.projects, |id| {
                self.is_mono_session(id)
            })
            .len(),
            OverviewTab::Documents => overview.documents.len(),
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
        let ink = if selected { fg } else { fg.opacity(0.45) };
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
            .text_size(px(12.0))
            .cursor_pointer()
            .text_color(ink)
            .map(|el| {
                if selected {
                    el.bg(fg.opacity(0.08))
                } else {
                    el.hover(move |s| s.bg(fg.opacity(0.05)).text_color(fg.opacity(0.8)))
                }
            })
            .when(!selected, |el| el.tooltip(Tooltip::text(label)))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        tab_icon(tab, ink).group_hover_color(group.clone(), fg.opacity(0.8)),
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
                                .text_size(px(10.0))
                                .line_height(px(16.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(gpui::white())
                                .child(count.to_string())
                        })),
                )
            })
    }

    /// Sessions: the threads in the Mono's projects, as the sidebar's
    /// cards would say them (model, title, project, when, status). One
    /// opens in the workspace.
    fn render_mono_sessions(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let name = mono.display_name();
        if mono.projects.is_empty() {
            return empty_state(
                OverviewTab::Sessions,
                format!(
                    "Give {name} a project to work on, and the sessions in it show up here."
                ),
                fg,
            );
        }
        let sessions = visible_sessions(&self.sessions, &mono.projects, |id| {
            self.is_mono_session(id)
        });
        if sessions.is_empty() {
            return empty_state(
                OverviewTab::Sessions,
                format!("No sessions yet in the projects {name} works on."),
                fg,
            );
        }
        let now = crate::app::now_ms();
        div()
            .children(sessions.into_iter().take(100).map(|session| {
                let id = session.id.clone();
                let title = crate::app::session_list::display_title(&session.title, &session.harness);
                let project = self.rail_project_label(&session.cwd);
                let status = if self.pending_permission_for(&session.id).is_some() {
                    "Needs approval".to_string()
                } else if self.is_agent_running_in(&session.id) {
                    "Working".to_string()
                } else {
                    crate::app::session_list::format_relative(session.updated_at, now)
                };
                let model = crate::harness::catalog::display_label(&session.harness, &session.model);
                list_row(
                    SharedString::from(format!("mono-session-{id}")),
                    HarnessIcon::new(&session.harness).size(px(14.0)).into_any_element(),
                    title,
                    format!("{project} · {model} · {status}"),
                    fg,
                )
                .tooltip(Tooltip::text("Open in workspace"))
                .on_click(cx.listener(move |this, _, _, cx| this.open_session(id.clone(), cx)))
            }))
            .into_any_element()
    }

    /// Documents: the files in the Mono's folder, newest first; one opens
    /// over the list.
    fn render_mono_documents(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let documents = &self.monos.overview.documents;
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
        let now = crate::app::now_ms();
        div()
            .children(documents.iter().map(|doc| {
                let name = doc.name.clone();
                let icon = if name.ends_with(".md") {
                    IconName::FileText
                } else {
                    IconName::File
                };
                list_row(
                    SharedString::from(format!("mono-document-{name}")),
                    Icon::new(icon)
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.6))
                        .into_any_element(),
                    name.clone(),
                    format!(
                        "{} · {}",
                        size_label(doc.bytes),
                        crate::ui::relative_time::since(doc.modified_ms, now)
                    ),
                    fg,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_mono_document(&name, cx)))
            }))
            .into_any_element()
    }

    /// A document over the list: its name and Back, then its text (a
    /// markdown file as the transcript renders it).
    fn render_mono_document(
        &self,
        document: &crate::app::mono_overview::OpenDocument,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(40.0))
            .pl_2()
            .pr_3()
            .border_b_1()
            .border_color(fg.opacity(STROKE))
            .child(
                IconButton::new("mono-document-back", IconName::ChevronLeft)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Back")
                    .on_click(cx.listener(|this, _, _, cx| this.close_mono_document(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(document.name.clone()),
            )
            .child(
                IconButton::new("mono-document-hide", IconName::PanelRight)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Hide overview")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mono_overview(cx))),
            );
        let content: AnyElement = match (&document.text, &document.error) {
            (_, Some(err)) => div()
                .text_size(px(12.0))
                .text_color(colors.danger)
                .child(err.clone())
                .into_any_element(),
            (None, None) => div()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.45))
                .child("Loading…")
                .into_any_element(),
            (Some(text), None) if text.trim().is_empty() => div()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.45))
                .child("Empty")
                .into_any_element(),
            (Some(text), None) if document.name.ends_with(".md") => {
                crate::ui::transcript::markdown::AgentMarkdown::new(
                    SharedString::from(format!("mono-document-md-{}", document.name)),
                    text.clone(),
                )
                .into_any_element()
            }
            (Some(text), None) => div()
                .font_family(cx.theme().mono_family.clone())
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(fg.opacity(0.85))
                .child(text.clone())
                .into_any_element(),
        };
        let scroll = div()
            .id("mono-document-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(content);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(crate::ui::scrollbar::Scrolled::new("mono-document-scrollbar", scroll))
            .into_any_element()
    }

    /// Habits: `habits.json`'s list (on ones first, soonest first), or
    /// MonoCode's empty state.
    fn render_mono_habits(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let Some(habits) = &self.monos.overview.habits else {
            // Blank while it loads, as MonoCode's list.
            return div().into_any_element();
        };
        if habits.is_empty() {
            return empty_state(
                OverviewTab::Habits,
                format!(
                    "Ask {} to put recurring work on a schedule, like a morning digest or a weekly report.",
                    mono.display_name()
                ),
                fg,
            );
        }
        div()
            .children(crate::monos::habits::ordered(habits).into_iter().map(|habit| {
                let line = if habit.enabled {
                    habit.schedule.label()
                } else {
                    format!("Paused · {}", habit.schedule.label())
                };
                list_row(
                    SharedString::from(format!("mono-habit-{}", habit.id)),
                    Icon::new(IconName::Clock)
                        .size(IconSize::Sm)
                        .color(fg.opacity(if habit.enabled { 0.6 } else { 0.3 }))
                        .into_any_element(),
                    habit.name.clone(),
                    line,
                    fg,
                )
                .tooltip(Tooltip::text(habit.instructions.clone()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_short() {
        assert_eq!(size_label(812), "812 B");
        assert_eq!(size_label(4300), "4.2 KB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MB");
    }
}
