//! Title bar (MonoCode `TitleBar.tsx`): the navigator's toggle while it is
//! hidden, a hand-rolled strip of workspace tabs, and at the right the
//! browser and terminal toggles and the inspector's.
//!
//! Each tab is 208px (shrinking to 104px before the strip scrolls) and shows
//! in one line its threads' harness icons (a spinner while one works, a
//! check when one finished unseen) and the focused thread's title, a close
//! button on hover, a tooltip naming the rest and a context menu.
//! Middle-click closes, tabs drag to reorder, and opening or closing a tab
//! sweeps its width.

mod mono;
pub mod tabs;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollEvent, ScrollHandle, SharedString, Styled, Window,
    div, ease_out_quint, point, prelude::*,
};

use crate::app::{BenCodeApp, InspectorMode, NEW_SESSION_TITLE};
use crate::ui::HarnessIcon;
use crate::ui::drag_drop::DraggedPane;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry};
use crate::ui::layout::WorkspaceTab;
use crate::ui::scale::px;
use crate::ui::sidebar::SessionDialog;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::ui::window_drag::claim_press;
use tabs::{
    CloseMany, TitleTab, closable, close_ids, next_unseen_finished, strip_overflow, tab_copy,
};

/// MonoCode `w-56 min-w-28`, in one line here.
const TAB_WIDTH: f32 = 208.0;
const TAB_MIN_WIDTH: f32 = 104.0;
const TAB_GAP: f32 = 2.0;
/// `pl-1.5 pr-2.5`.
const STRIP_PADDING: f32 = 16.0;
/// MonoCode `--motion-tab-close-duration`.
const TAB_MOTION: Duration = Duration::from_millis(200);
/// Room for the macOS window buttons when nothing else on the left holds it.
const TRAFFIC_LIGHT_SPACE: gpui::Pixels = gpui::px(72.0);

/// MonoCode's tab menu is 244px wide.
const TAB_MENU_WIDTH: f32 = 244.0;

/// The open tab context menu.
struct TabMenu {
    tab_id: String,
    position: gpui::Point<gpui::Pixels>,
    active: usize,
}

/// A tab sweeping shut after it closed.
struct ClosingTab {
    tab: TitleTab,
    index: usize,
    width: f32,
}

/// What the strip remembers between frames.
#[derive(Default)]
pub struct TitleStrip {
    pub scroll: ScrollHandle,
    /// Tabs drawn last frame; a new id sweeps open.
    seen: HashSet<String>,
    opening: HashMap<String, Instant>,
    closing: Vec<ClosingTab>,
    last_active: Option<String>,
    /// Threads working last frame, and those that finished unseen.
    busy: HashSet<String>,
    pub(crate) unseen_finished: HashSet<String>,
    window_title: String,
    menu: Option<TabMenu>,
    /// While a tab is dragged along the strip: it and the place it would
    /// land (MonoCode `useAnimatedReorder`).
    reorder: Option<(String, usize)>,
    /// Tab order drawn last frame, and the tabs sliding to a new place:
    /// their start offset, a generation for the animation id, and when.
    order: Vec<String>,
    slides: HashMap<String, (f32, u64, Instant)>,
    slide_seq: u64,
}

/// Payload while a title tab is dragged to a new place.
#[derive(Clone)]
pub struct DraggedTitleTab {
    pub id: String,
    headline: String,
}

impl Render for DraggedTitleTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        div()
            .h(px(28.0))
            .w(px(TAB_WIDTH))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.0))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_lg()
            .text_size(Text::Md.px())
            .text_color(colors.fg)
            .child(div().truncate().child(self.headline.clone()))
    }
}

fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map_or_else(|| cwd.to_string(), |n| n.to_string_lossy().into_owned())
}

impl BenCodeApp {
    fn conversation_title(&self, id: &str) -> Option<String> {
        let session = self.sessions.iter().find(|s| s.id == id)?;
        let title = session.title.trim();
        Some(if title == NEW_SESSION_TITLE {
            String::new()
        } else {
            title.to_string()
        })
    }

    /// MonoCode `toTitleTab`.
    fn title_tab(&self, tab: &WorkspaceTab) -> TitleTab {
        let ids = tab.leaf_ids();
        let mut ordered: Vec<&str> = vec![tab.focused.as_str()];
        ordered.extend(
            ids.iter()
                .map(String::as_str)
                .filter(|id| *id != tab.focused),
        );
        let sessions: Vec<_> = ordered
            .iter()
            .filter_map(|id| self.sessions.iter().find(|s| s.id == *id))
            .collect();
        let mut out = TitleTab {
            id: tab.id.clone(),
            project: sessions
                .first()
                .map_or_else(|| "~".to_string(), |s| project_name(&s.cwd)),
            title: self.conversation_title(&tab.focused).unwrap_or_default(),
            more: sessions
                .iter()
                .skip(1)
                .filter_map(|s| self.conversation_title(&s.id))
                .filter(|t| !t.is_empty())
                .collect(),
            session_count: sessions.len(),
            multi_pane: ids.len() > 1,
            blank: ids.len() == 1
                && sessions
                    .first()
                    .is_none_or(|s| !s.blocks.iter().any(|b| b.role == "user")),
            ..Default::default()
        };
        for session in &sessions {
            let harness = session.harness.clone();
            let working = self.is_agent_running_in(&session.id)
                && self.pending_permission_for(&session.id).is_none();
            if working && !out.busy.contains(&harness) {
                out.busy.push(harness.clone());
            }
            if self.title_strip.unseen_finished.contains(&session.id)
                && !out.done.contains(&harness)
            {
                out.done.push(harness.clone());
            }
            if !out.harnesses.contains(&harness) {
                out.harnesses.push(harness);
            }
        }
        out
    }

    fn title_tabs(&self) -> Vec<TitleTab> {
        self.deck_tabs()
            .into_iter()
            .map(|tab| self.title_tab(tab))
            .collect()
    }

    /// The width each tab gets in the strip as it stands.
    fn title_tab_width(&self, count: usize) -> f32 {
        let strip = crate::ui::scale::logical(self.title_strip.scroll.bounds().size.width);
        if strip <= 0.0 || count == 0 {
            return TAB_WIDTH;
        }
        let room = strip - STRIP_PADDING - TAB_GAP * count.saturating_sub(1) as f32;
        (room / count as f32).clamp(TAB_MIN_WIDTH, TAB_WIDTH)
    }

    /// Keeps per-frame strip state: unseen finishes, opening sweeps, the
    /// active tab scrolled into view, and the window title.
    fn sync_title_strip(&mut self, tabs: &[TitleTab], window: &mut Window) {
        let busy: HashSet<String> = self
            .sessions
            .iter()
            .filter(|s| {
                self.is_agent_running_in(&s.id) && self.pending_permission_for(&s.id).is_none()
            })
            .map(|s| s.id.clone())
            .collect();
        let strip = &mut self.title_strip;
        strip.unseen_finished = next_unseen_finished(
            &strip.busy,
            &busy,
            &strip.unseen_finished,
            self.selected_session_id.as_deref(),
        );
        strip.busy = busy;

        let now = Instant::now();
        let first_frame = strip.seen.is_empty();
        for tab in tabs {
            if !first_frame && !strip.seen.contains(&tab.id) {
                strip.opening.insert(tab.id.clone(), now);
            }
        }
        strip.seen = tabs.iter().map(|t| t.id.clone()).collect();

        // Tabs whose place changed slide there from where they were.
        let ids: Vec<String> = tabs.iter().map(|t| t.id.clone()).collect();
        let order = tabs::preview_order(
            &ids,
            strip.reorder.as_ref().map(|(id, to)| (id.as_str(), *to)),
        );
        let pitch = {
            let strip_w = crate::ui::scale::logical(strip.scroll.bounds().size.width);
            let count = tabs.len().max(1);
            if strip_w <= 0.0 {
                TAB_WIDTH
            } else {
                ((strip_w - STRIP_PADDING - TAB_GAP * (count - 1) as f32) / count as f32)
                    .clamp(TAB_MIN_WIDTH, TAB_WIDTH)
            }
        } + TAB_GAP;
        for (to, id) in order.iter().enumerate() {
            let from = strip.order.iter().position(|o| o == id);
            if let Some(from) = from
                && from != to
                && !first_frame
            {
                strip.slide_seq += 1;
                strip.slides.insert(
                    id.clone(),
                    ((from as f32 - to as f32) * pitch, strip.slide_seq, now),
                );
            }
        }
        strip
            .slides
            .retain(|_, (_, _, at)| at.elapsed() < TAB_MOTION);
        strip.order = order;
        strip.opening.retain(|_, at| at.elapsed() < TAB_MOTION);

        let active = self.tabs.active_id().map(str::to_string);
        if active != strip.last_active {
            if let Some(ix) = active
                .as_deref()
                .and_then(|id| tabs.iter().position(|t| t.id == id))
            {
                strip.scroll.scroll_to_item(ix);
            }
            strip.last_active = active.clone();
        }

        // MonoCode `systemTitle`: "project — BenCode".
        let title = active
            .and_then(|id| tabs.iter().find(|t| t.id == id))
            .map_or_else(
                || "BenCode".to_string(),
                |t| format!("{} — BenCode", t.project),
            );
        if title != strip.window_title {
            window.set_window_title(&title);
            strip.window_title = title;
        }
    }

    /// Closes a tab from the strip, leaving a ghost that sweeps shut.
    fn close_title_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let tabs = self.title_tabs();
        let Some(index) = tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let width = self.title_tab_width(tabs.len());
        self.close_tab(id, cx);
        if self.deck_tabs().iter().any(|t| t.id == id) {
            return;
        }
        self.title_strip.closing.push(ClosingTab {
            tab: tabs[index].clone(),
            index,
            width,
        });
        let gone = id.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TAB_MOTION).await;
            let removed = this.update(cx, |app, cx| {
                app.title_strip.closing.retain(|c| c.tab.id != gone);
                cx.notify();
            });
            if let Err(err) = removed {
                log::debug!("tab sweep after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode `onCloseTabs`: closes `ids` and lands on `keep`.
    fn close_title_tabs(&mut self, ids: &[String], keep: &str, cx: &mut Context<Self>) {
        for id in ids {
            self.close_title_tab(id, cx);
        }
        self.switch_tab(keep, cx);
    }

    /// MonoCode `onCloseOtherTabs` (⌥⌘T): every tab of the strip but the
    /// active one.
    pub(crate) fn close_other_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.tabs.active_id().map(str::to_string) else {
            return;
        };
        let tabs = self.title_tabs();
        self.close_title_tabs(&close_ids(&tabs, &active, CloseMany::Others), &active, cx);
    }

    /// MonoCode `onCloseAllTabs` (⇧⌘W): the files open beside the chat
    /// first; with none open, every tab of the strip.
    pub(crate) fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        if !self.file_pane.is_empty() {
            let mut last = None;
            while let Some(key) = self.file_pane.active_key().map(str::to_string) {
                // A tab that stays (an unsaved file asking first) ends the pass.
                if last.as_deref() == Some(key.as_str()) || !self.close_active_pane_tab(cx) {
                    break;
                }
                last = Some(key);
            }
            return;
        }
        let Some(active) = self.tabs.active_id().map(str::to_string) else {
            return;
        };
        self.close_other_tabs(cx);
        self.close_tab(&active, cx);
    }

    /// MonoCode `onArchiveTitleTab`: archives the tab's threads and closes it.
    fn archive_title_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(sessions) = self
            .tabs
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.leaf_ids())
        else {
            return;
        };
        for session in &sessions {
            if self
                .sessions
                .iter()
                .any(|s| s.id == *session && !s.archived)
            {
                self.toggle_archive_session(session, cx);
            }
        }
        self.close_title_tab(id, cx);
    }

    /// MonoCode's tab menu rows for `tab_id`, as the strip stands now.
    fn tab_menu_entries(&self, tab_id: &str) -> Vec<MenuEntry> {
        let tabs = self.title_tabs();
        let Some(tab) = tabs.iter().find(|t| t.id == tab_id) else {
            return Vec::new();
        };
        let none = |action| close_ids(&tabs, tab_id, action).is_empty();
        let mut entries = vec![
            MenuEntry::Item(
                MenuAction::new("close", "Close Tab")
                    .shortcut("⌘W")
                    .disabled(!closable(tab, tabs.len())),
            ),
            MenuEntry::Separator,
            MenuEntry::Item(
                MenuAction::new("others", "Close Other Tabs").disabled(none(CloseMany::Others)),
            ),
            MenuEntry::Item(
                MenuAction::new("right", "Close Tabs to the Right")
                    .disabled(none(CloseMany::Right)),
            ),
            MenuEntry::Item(
                MenuAction::new("left", "Close Tabs to the Left").disabled(none(CloseMany::Left)),
            ),
        ];
        if tab.session_count > 0 {
            let many = tab.session_count > 1;
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Item(MenuAction::new("archive", "Archive").description(
                    many.then(|| format!("All {} conversations in this tab", tab.session_count)),
                )),
                MenuEntry::Item(
                    MenuAction::new("delete", "Delete")
                        .description(many.then(|| {
                            format!(
                                "Permanently delete all {} conversations in this tab",
                                tab.session_count
                            )
                        }))
                        .danger(),
                ),
            ]);
        }
        entries
    }

    /// Right-click on a tab: its menu at the pointer, first row lit.
    fn open_tab_menu(
        &mut self,
        tab_id: &str,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let entries = self.tab_menu_entries(tab_id);
        self.title_strip.menu = Some(TabMenu {
            tab_id: tab_id.to_string(),
            position,
            active: explorer_menu::first_item(&entries),
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub fn tab_menu_open(&self) -> bool {
        self.title_strip.menu.is_some()
    }

    pub fn close_tab_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.title_strip.menu.take().is_none() {
            return false;
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    /// MonoCode `onPickTabMenu`.
    fn pick_tab_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.title_strip.menu.take() else {
            return;
        };
        let entries = self.tab_menu_entries(&menu.tab_id);
        let Some(id) = explorer_menu::pick(&entries, index) else {
            self.title_strip.menu = Some(menu);
            return;
        };
        self.refocus_prompt(cx);
        let tabs = self.title_tabs();
        let target = menu.tab_id;
        match id {
            "close" => self.close_title_tab(&target, cx),
            "others" => {
                self.close_title_tabs(&close_ids(&tabs, &target, CloseMany::Others), &target, cx)
            }
            "right" => {
                self.close_title_tabs(&close_ids(&tabs, &target, CloseMany::Right), &target, cx)
            }
            "left" => {
                self.close_title_tabs(&close_ids(&tabs, &target, CloseMany::Left), &target, cx)
            }
            "archive" => self.archive_title_tab(&target, cx),
            "delete" => {
                let sessions = self
                    .tabs
                    .tabs()
                    .iter()
                    .find(|t| t.id == target)
                    .map(|t| t.leaf_ids())
                    .unwrap_or_default();
                self.session_dialog = Some(match sessions.as_slice() {
                    [one] => SessionDialog::Delete(one.clone()),
                    _ => SessionDialog::DeleteMany(sessions),
                });
            }
            _ => {}
        }
        cx.notify();
    }

    /// Keys while the tab menu holds focus (MonoCode `ExplorerMenu.onMenuKey`).
    pub fn tab_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.title_strip.menu.as_ref() else {
            return false;
        };
        let entries = self.tab_menu_entries(&menu.tab_id);
        let active = menu.active;
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                if let Some(menu) = self.title_strip.menu.as_mut() {
                    menu.active = explorer_menu::step(&entries, active, dir);
                }
            }
            "enter" | "space" => self.pick_tab_menu(active, cx),
            "escape" => {
                self.close_tab_menu(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn render_tab_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.title_strip.menu.as_ref()?;
        let entries = self.tab_menu_entries(&menu.tab_id);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            explorer_menu::MenuView {
                id: "title-tab-menu",
                entries: &entries,
                active: menu.active,
                place: explorer_menu::MenuPlace::At(menu.position),
                width: TAB_MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.title_strip.menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("tab menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_tab_menu(ix, cx)) {
                    log::debug!("tab menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                let closed = close_app.update(cx, |this, cx| {
                    this.close_tab_menu(cx);
                });
                if let Err(err) = closed {
                    log::debug!("tab menu dismiss after app drop: {err:#}");
                }
            },
            cx,
        ))
    }

    /// MonoCode `TabHarnesses`: up to three icons, overlapping; a spinner
    /// for one at work, a check for one done unseen.
    fn tab_harnesses(&self, tab: &TitleTab, active: bool, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let shown = tab.harnesses.iter().take(3);
        let extra = tab.harnesses.len().saturating_sub(3);
        // Sized by hand: taffy gives a row no width at all once a child has
        // a negative margin, and the headline would start under the icons.
        let stack_width = 14.0 + 12.0 * shown.len().saturating_sub(1) as f32;
        // The dragged tab's slot is see-through while it rides the pointer;
        // a spinner is drawn over the regions (`ui/marks.rs`) and would
        // stay behind in the gap, so it shows the harness instead.
        let dragged = self
            .title_strip
            .reorder
            .as_ref()
            .is_some_and(|(id, _)| *id == tab.id);
        let stack = div()
            .flex()
            .flex_none()
            .items_center()
            .w(px(stack_width))
            .children(shown.enumerate().map(|(ix, harness)| {
                let slot = div()
                    .size(px(14.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(ix > 0, |el| el.ml(px(-2.0)));
                if tab.busy.contains(harness) && !dragged {
                    slot.child(crate::ui::spinner::terminal_spinner(colors.accent, cx))
                        .into_any_element()
                } else if tab.done.contains(harness) {
                    slot.child(
                        Icon::new(IconName::CircleCheck)
                            .size(IconSize::Xs)
                            .color(colors.success),
                    )
                    .into_any_element()
                } else {
                    slot.when(!active, |el| el.opacity(0.55))
                        .child(HarnessIcon::new(harness.as_str()).size(px(14.0)))
                        .into_any_element()
                }
            }));
        div()
            .flex()
            .flex_none()
            .items_center()
            .child(stack)
            .when(extra > 0, |el| {
                el.child(
                    div()
                        .pl_0p5()
                        .text_size(Text::Xs.px())
                        .text_color(if active {
                            colors.fg
                        } else {
                            colors.fg.ink(Ink::Secondary)
                        })
                        .child(format!("+{extra}")),
                )
            })
            .into_any_element()
    }

    /// One tab: icons, the headline, the hover close button.
    fn render_title_tab(
        &self,
        tab: &TitleTab,
        tabs: &[TitleTab],
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let active = self.tabs.active_id() == Some(tab.id.as_str());
        let can_close = closable(tab, tabs.len());
        let (headline, tooltip) = tab_copy(tab);
        let group = SharedString::from(format!("title-tab-{}", tab.id));
        let hover_bg = colors.fg.fill(Fill::Hover);
        let fg = colors.fg;
        let (select_id, close_id, middle_id) = (tab.id.clone(), tab.id.clone(), tab.id.clone());
        let drag = DraggedTitleTab {
            id: tab.id.clone(),
            headline: headline.clone(),
        };
        let text = div()
            .flex_1()
            .min_w_0()
            .truncate()
            .line_height(px(15.0))
            .text_size(Text::Base.px())
            .when(active, |el| el.font_weight(FontWeight::MEDIUM))
            .child(headline);
        let button = div()
            .id(SharedString::from(format!("title-tab-btn-{}", tab.id)))
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .h(px(28.0))
            .items_center()
            .gap(px(6.0))
            .px_2()
            .when(can_close, |el| el.pr(px(28.0)))
            .when(!can_close, |el| el.pr(px(10.0)))
            .rounded(px(6.0))
            .map(|el| {
                if active {
                    el.bg(colors.active).text_color(fg)
                } else {
                    el.text_color(fg.ink(Ink::Secondary))
                        .hover(move |s| s.bg(hover_bg).text_color(fg))
                }
            })
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _, _, cx| this.switch_tab(&select_id, cx)))
            .child(self.tab_harnesses(tab, active, cx))
            .child(text);
        let close = can_close.then(|| {
            let hover = colors.fg.fill(Fill::Strong);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_1()
                .flex()
                .items_center()
                .child(
                    div()
                        .id(SharedString::from(format!("title-tab-close-{}", tab.id)))
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .opacity(0.0)
                        .group_hover(group.clone(), |s| s.opacity(1.0))
                        .hover(move |s| s.bg(hover))
                        .tooltip(Tooltip::text("Close Tab"))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_title_tab(&close_id, cx);
                        }))
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Xs)
                                .color(colors.fg.ink(Ink::Secondary)),
                        ),
                )
        });
        let item = div()
            .id(SharedString::from(format!("title-tab-{}", tab.id)))
            .group(group)
            .relative()
            .flex()
            .h_full()
            .w_full()
            .min_w_0()
            .items_center()
            .on_mouse_down(MouseButton::Right, {
                let id = tab.id.clone();
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_tab_menu(&id, event.position, cx);
                })
            })
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, _, cx| {
                    if can_close {
                        this.close_title_tab(&middle_id, cx);
                    }
                }),
            )
            // MonoCode `canDrag`: a lone tab has nowhere to go.
            .when(tabs.len() > 1, |el| {
                el.on_drag(drag, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            })
            .child(button)
            .children(close);
        // A press on a tab is the tab's, never the window's.
        claim_press(item).into_any_element()
    }

    /// The tab held over the strip lands where it is previewed.
    fn finish_tab_reorder(&mut self, cx: &mut Context<Self>) {
        let Some((id, to)) = self.title_strip.reorder.take() else {
            return;
        };
        let from = self.deck_tabs().iter().position(|t| t.id == id);
        if let Some(from) = from
            && from != to
        {
            self.reorder_open_tabs(from, to, cx);
        }
        cx.notify();
    }

    /// Follows a tab dragged along the strip, previewing where it lands.
    fn track_tab_reorder(
        &mut self,
        event: &gpui::DragMoveEvent<DraggedTitleTab>,
        cx: &mut Context<Self>,
    ) {
        let id = event.drag(cx).id.clone();
        let inside = event.bounds.contains(&event.event.position);
        let next = inside.then(|| {
            let count = self.deck_tabs().len();
            let width = self.title_tab_width(count);
            let scroll = crate::ui::scale::logical(self.title_strip.scroll.offset().x);
            let x = crate::ui::scale::logical(event.event.position.x - event.bounds.origin.x)
                - 6.0
                - scroll;
            (id, tabs::slot_at(x, width, TAB_GAP, count))
        });
        if next != self.title_strip.reorder {
            self.title_strip.reorder = next;
            cx.notify();
        }
    }

    /// A tab's slot in the strip; a new one sweeps open from nothing.
    fn title_tab_slot(
        &self,
        tab: &TitleTab,
        tabs: &[TitleTab],
        width: f32,
        cx: &Context<Self>,
    ) -> AnyElement {
        let slot = div()
            .flex()
            .flex_none()
            .h_full()
            .w(px(width))
            .min_w(px(TAB_MIN_WIDTH.min(width)))
            // A column, so the context-menu host stretches to the slot
            // instead of growing to its text.
            .flex_col()
            .justify_center()
            .child(self.render_title_tab(tab, tabs, cx));
        let dragged = self
            .title_strip
            .reorder
            .as_ref()
            .is_some_and(|(id, _)| *id == tab.id);
        // The dragged tab rides the pointer; its slot holds the gap open.
        let slot = slot.when(dragged, |el| el.opacity(0.0));
        let slide = self
            .title_strip
            .slides
            .get(&tab.id)
            .filter(|(_, _, at)| at.elapsed() < TAB_MOTION)
            .copied();
        if let (Some((offset, seq, _)), false) =
            (slide, self.title_strip.opening.contains_key(&tab.id))
        {
            return slot
                .relative()
                .with_animation(
                    SharedString::from(format!("title-tab-slide-{}-{seq}", tab.id)),
                    Animation::new(TAB_MOTION).with_easing(ease_out_quint()),
                    move |el, delta| el.left(px(offset * (1.0 - delta))),
                )
                .into_any_element();
        }
        match self.title_strip.opening.get(&tab.id) {
            Some(_) => slot
                .overflow_hidden()
                .with_animation(
                    SharedString::from(format!("title-tab-open-{}", tab.id)),
                    Animation::new(TAB_MOTION).with_easing(ease_out_quint()),
                    move |el, delta| el.w(px(width * delta)).min_w(px(0.0)),
                )
                .into_any_element(),
            None => slot.into_any_element(),
        }
    }

    /// The ghost of a closed tab, sweeping shut.
    fn closing_tab_slot(&self, closing: &ClosingTab, cx: &Context<Self>) -> AnyElement {
        let (headline, _) = tab_copy(&closing.tab);
        let colors = &cx.theme().colors;
        let width = closing.width;
        div()
            .flex()
            .flex_none()
            .h_full()
            .w(px(width))
            .overflow_hidden()
            .items_center()
            .child(
                div()
                    .h(px(28.0))
                    .w(px(width))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .text_size(Text::Md.px())
                    .text_color(colors.fg.ink(Ink::Secondary))
                    .child(div().truncate().child(headline)),
            )
            .with_animation(
                SharedString::from(format!("title-tab-close-sweep-{}", closing.tab.id)),
                Animation::new(TAB_MOTION).with_easing(ease_out_quint()),
                move |el, delta| el.w(px(width * (1.0 - delta))).opacity(1.0 - delta),
            )
            .into_any_element()
    }

    /// MonoCode `TabStripChevron`.
    fn strip_chevron(&self, left: bool, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let hover = colors.fg.fill(Fill::Bold);
        claim_press(div())
            .id(if left {
                "title-strip-left"
            } else {
                "title-strip-right"
            })
            .absolute()
            .top_0()
            .bottom_0()
            .when(left, |el| el.left_1())
            .when(!left, |el| el.right_1())
            .flex()
            .items_center()
            .child(
                div()
                    .id(if left {
                        "title-strip-left-btn"
                    } else {
                        "title-strip-right-btn"
                    })
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .bg(colors.surface)
                    .border_1()
                    .border_color(colors.fg.fill(Fill::Strong))
                    .hover(move |s| s.bg(hover))
                    .tooltip(Tooltip::text(if left {
                        "Scroll tabs left"
                    } else {
                        "Scroll tabs right"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let scroll = &this.title_strip.scroll;
                        let visible = crate::ui::scale::logical(scroll.bounds().size.width);
                        let step = (visible * 0.6).max(112.0) * if left { 1.0 } else { -1.0 };
                        let max = crate::ui::scale::logical(scroll.max_offset().x);
                        let x =
                            (crate::ui::scale::logical(scroll.offset().x) + step).clamp(-max, 0.0);
                        scroll.set_offset(point(px(x), px(0.0)));
                        cx.notify();
                    }))
                    .child(
                        Icon::new(if left {
                            IconName::ChevronLeft
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Xs)
                        .color(colors.fg.ink(Ink::Body)),
                    ),
            )
            .into_any_element()
    }

    /// The strip itself, scrolling sideways when the tabs do not fit.
    fn render_tab_strip(&self, tabs: &[TitleTab], cx: &Context<Self>) -> AnyElement {
        let width = self.title_tab_width(tabs.len() + self.title_strip.closing.len());
        let mut slots: Vec<AnyElement> = self
            .title_strip
            .order
            .iter()
            .filter_map(|id| tabs.iter().find(|t| t.id == *id))
            .map(|tab| self.title_tab_slot(tab, tabs, width, cx))
            .collect();
        for closing in &self.title_strip.closing {
            let at = closing.index.min(slots.len());
            slots.insert(at, self.closing_tab_slot(closing, cx));
        }
        let scroll = &self.title_strip.scroll;
        let (left, right) = strip_overflow(
            -crate::ui::scale::logical(scroll.offset().x),
            crate::ui::scale::logical(scroll.max_offset().x),
        );
        let drop_wash = cx.theme().colors.hover;
        div()
            .id("title-tab-drop")
            .relative()
            .h_full()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .drag_over::<DraggedPane>(move |style, _, _, _| style.bg(drop_wash))
            .on_drop(cx.listener(|this, dragged: &DraggedPane, _, cx| {
                this.detach_pane_to_new_tab(&dragged.session_id, cx);
            }))
            .on_drag_move::<DraggedTitleTab>(cx.listener(|this, event, _, cx| {
                this.track_tab_reorder(event, cx);
            }))
            .on_drop(cx.listener(|this, _: &DraggedTitleTab, _, cx| {
                this.finish_tab_reorder(cx);
            }))
            // A vertical wheel scrolls the strip sideways.
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.0));
                let scroll = &this.title_strip.scroll;
                let max = crate::ui::scale::logical(scroll.max_offset().x);
                if max <= 0.0
                    || crate::ui::scale::logical(delta.x) != 0.0
                    || crate::ui::scale::logical(delta.y) == 0.0
                {
                    return;
                }
                let x = (crate::ui::scale::logical(scroll.offset().x)
                    + crate::ui::scale::logical(delta.y))
                .clamp(-max, 0.0);
                scroll.set_offset(point(px(x), px(0.0)));
                cx.notify();
            }))
            .child(
                div()
                    .id("title-tab-strip")
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(TAB_GAP))
                    .pl(px(6.0))
                    .pr(px(10.0))
                    .overflow_x_scroll()
                    .track_scroll(scroll)
                    .children(slots),
            )
            .when(left, |el| el.child(self.strip_chevron(true, cx)))
            .when(right, |el| el.child(self.strip_chevron(false, cx)))
            .into_any_element()
    }

    pub fn render_titlebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tabs = self.title_tabs();
        if !cx.has_active_drag() {
            self.title_strip.reorder = None;
        }
        self.sync_title_strip(&tabs, window);
        // MonoCode's title bar takes no glass tint: the blur shows bare.
        let glass = self.glass(cx);
        let theme = cx.theme();
        let colors = &theme.colors;
        self.window_drag_region(div(), cx)
            .flex()
            .items_stretch()
            .h(crate::ui::scale::px(crate::ui::sidebar::TITLEBAR_HEIGHT)) // MonoCode `h-10`
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(colors.border)
            .bg(glass.fill(colors.bg))
            .child(self.render_titlebar_leading(cx))
            .child(self.render_tab_strip(&tabs, cx))
            .children(self.render_tab_menu(cx))
            .child(self.render_titlebar_trailing(cx))
    }

    /// With the navigator hidden the title bar takes over its traffic-light
    /// space, its toggle, Back / Forward and New thread (MonoCode
    /// `TitleBar.tsx:856-941`), and the agents at work.
    fn render_titlebar_leading(&self, cx: &Context<Self>) -> impl IntoElement {
        // The icon rail expands the navigator itself; with the title bar
        // above it (MonoCode `compactRail`: `pl-[70px]` then `TabVisitNav`)
        // only the traffic-light gap and Back / Forward lead.
        let compact = self.compact_rail_active();
        let rail_hidden = !self.is_rail_open && !compact;
        claim_press(div())
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .when(rail_hidden, |el| el.px_1p5())
            .when(self.compact_title_bar(), |el| {
                el.pl(gpui::px(70.0))
                    .children(self.history_buttons("titlebar", cx))
            })
            .when(rail_hidden && cfg!(target_os = "macos"), |el| {
                el.child(div().w(TRAFFIC_LIGHT_SPACE))
            })
            .when(rail_hidden, |el| {
                el.child(
                    IconButton::new("titlebar-toggle-projects", IconName::PanelLeft)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip("Show Navigator (⌘B)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.is_rail_open = true;
                            cx.notify();
                        })),
                )
                .children(self.history_buttons("titlebar", cx))
                .child(
                    IconButton::new("titlebar-new-session", IconName::Plus)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip("New thread (⌘T)")
                        .on_click(cx.listener(|this, _, _, cx| this.create_new_session(cx))),
                )
                .children(self.render_agents_badge(false, cx))
            })
    }

    /// The window's status at the right: the browser and terminal toggles,
    /// then what the inspector shows.
    fn render_titlebar_trailing(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let projectless = self.current_cwd.trim().is_empty();
        claim_press(div())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .pl_2()
            .pr(px(8.0))
            .child(self.render_browser_toggle(cx))
            .child(self.render_terminal_status(cx))
            .when(!projectless, |el| {
                el.child(
                    div()
                        .flex_none()
                        .w(px(1.0))
                        .h(px(16.0))
                        .mx(px(6.0))
                        .bg(fg.fill(Fill::Strong)),
                )
                .child(self.render_inspector_toggles(cx))
            })
    }

    /// Changes (with the working tree's diff stat) and Explorer: each shows
    /// the inspector on itself, or hides it when it already does.
    fn render_inspector_toggles(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (added, removed) = self
            .git_status
            .staged
            .iter()
            .chain(&self.git_status.unstaged)
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let toggle = |id: &'static str, mode: InspectorMode, tip: &'static str| {
            let active = self.inspector_open && self.inspector_mode == mode;
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .h(px(24.0))
                .px(px(7.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(Text::Sm.px())
                .map(|el| {
                    if active {
                        el.bg(colors.active).text_color(fg)
                    } else {
                        el.text_color(fg.ink(Ink::Secondary))
                            .hover(move |s| s.bg(fg.fill(Fill::Strong)).text_color(fg))
                    }
                })
                .tooltip(Tooltip::text(tip))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_inspector_mode(mode, cx)))
        };
        let changes = toggle("toolbar-changes", InspectorMode::Changes, "Changes (⌃⇧G)")
            .child(
                crate::ui::icons::ExtraIcon::FileDiff
                    .icon()
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Secondary)),
            )
            .when(added + removed > 0, |el| {
                el.child(
                    crate::ui::diff_counts::diff_counts(
                        added,
                        removed,
                        crate::ui::appearance::diff_colors(cx),
                    )
                    .gap_1()
                    .text_size(Text::Sm.px()),
                )
            });
        let files = toggle("toolbar-files", InspectorMode::Files, "Explorer (⇧⌘E)").child(
            crate::ui::icons::ExtraIcon::FolderTree
                .icon()
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Secondary)),
        );
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .child(changes)
            .child(files)
    }
}
