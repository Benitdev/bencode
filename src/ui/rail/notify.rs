//! MonoCode's notification mutes on the rail: the Inbox row's menu
//! (`InboxNotificationMenu`) and the "Choose date and time" step
//! (`NotificationMuteDatePicker`). BenCode posts no system notifications
//! yet, so a mute is kept (with MonoCode's meaning) and shown on the rail.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::DateTimePicker;
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseDownEvent,
    ParentElement, Pixels, Point, Styled, anchored, deferred, div, px, rgb,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

use super::menu_view::{SUBMENU_WIDTH, submenu_bounds};
use super::model::notification_id;
use super::state::{MutePicker, SubmenuKind};
use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback_with;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuView};

/// MonoCode's `Popover width={280}` around the date picker.
const PICKER_WIDTH: f32 = 280.0;
/// `ExplorerMenu`'s inset (`p-1` in a 1px border) and its rows' heights.
const MENU_INSET: f32 = 5.0;
const ROW: f32 = 28.0;
const SEPARATOR: f32 = 9.0;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
/// MonoCode `text-red-400`.
const RED_400: u32 = 0xf87171;

/// Local wall time of `ms`.
fn local_datetime(ms: i64) -> Option<DateTime> {
    let t = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(t.to_zoned(TimeZone::system()).datetime())
}

/// `value` read as local time, in ms.
fn local_ms(value: DateTime) -> Option<i64> {
    let zoned = value.to_zoned(TimeZone::system()).ok()?;
    Some(zoned.timestamp().as_millisecond())
}

impl BenCodeApp {
    /// Every rail project's notification id (MonoCode `useNotificationProjects`).
    pub(super) fn rail_notification_ids(&self) -> Vec<String> {
        self.recent_projects
            .iter()
            .filter(|p| !self.settings.rail.is_archived(p))
            .map(|p| notification_id(p))
            .collect()
    }

    fn muted_notification_ids(&self) -> Vec<String> {
        let now = crate::app::now_ms();
        self.rail_notification_ids()
            .into_iter()
            .filter(|id| {
                self.settings
                    .rail
                    .project_notifications
                    .get(id)
                    .is_some_and(|p| p.is_muted(now))
            })
            .collect()
    }

    /// MonoCode `InboxNotificationMenu` items (no "Notification settings…":
    /// BenCode has no such page).
    pub(super) fn inbox_menu_entries(&self) -> Vec<MenuEntry> {
        vec![
            MenuEntry::Item(MenuAction::new("read-all", "Mark all as read").disabled(!self.inbox_has_unseen())),
            MenuEntry::Separator,
            MenuEntry::Item(
                MenuAction::new("mute", "Mute all projects")
                    .submenu()
                    .disabled(self.rail_notification_ids().is_empty()),
            ),
            MenuEntry::Item(
                MenuAction::new("resume", "Resume muted projects")
                    .disabled(self.muted_notification_ids().is_empty()),
            ),
        ]
    }

    /// Right-click on Inbox (MonoCode `onOpenContextMenu`).
    pub(super) fn open_inbox_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.close_sidebar_menu(cx);
        self.rail_ui.inbox_active = explorer_menu::first_item(&self.inbox_menu_entries());
        self.rail_ui.inbox_menu = Some(at);
        self.focus_composer_menu(cx);
        cx.notify();
    }

    /// Where "Mute all projects" sits, for its submenu.
    fn inbox_mute_row(&self) -> Option<Bounds<Pixels>> {
        let at = self.rail_ui.inbox_menu?;
        let top = at.y + px(MENU_INSET + ROW + SEPARATOR);
        Some(Bounds::new(
            Point::new(at.x + px(MENU_INSET), top),
            gpui::size(px(SUBMENU_WIDTH - MENU_INSET * 2.0), px(ROW)),
        ))
    }

    fn hover_inbox_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.rail_ui.inbox_active = ix;
        let entries = self.inbox_menu_entries();
        let on_mute = matches!(
            entries.get(ix),
            Some(MenuEntry::Item(MenuAction { id: "mute", disabled: false, .. }))
        );
        match (on_mute, self.inbox_mute_row()) {
            (true, Some(row)) => {
                let position = Point::new(row.right() + px(4.0), row.top() - px(MENU_INSET));
                if self.rail_ui.submenu.is_none() {
                    let active = explorer_menu::first_item(&self.rail_submenu_entries(SubmenuKind::InboxMute));
                    self.rail_ui.submenu = Some(super::state::RailSubmenu {
                        kind: SubmenuKind::InboxMute,
                        position,
                        active,
                    });
                }
            }
            _ => self.rail_ui.submenu = None,
        }
        cx.notify();
    }

    pub(super) fn pick_inbox_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        let entries = self.inbox_menu_entries();
        let Some(action) = explorer_menu::pick_action(&entries, ix).cloned() else {
            return;
        };
        match action.id {
            "read-all" => {
                self.mark_all_inbox_seen(cx);
                self.close_rail_menu(cx);
            }
            "mute" => self.hover_inbox_menu(ix, cx),
            "resume" => {
                let ids = self.muted_notification_ids();
                let now = crate::app::now_ms();
                self.update_rail_prefs(|prefs| prefs.with_mute(&ids, None, now), cx);
                self.close_rail_menu(cx);
            }
            _ => {}
        }
    }

    pub(super) fn render_inbox_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let at = self.rail_ui.inbox_menu?;
        let entries = self.inbox_menu_entries();
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "rail-inbox-menu",
                entries: &entries,
                active: self.rail_ui.inbox_active,
                place: MenuPlace::At(at),
                width: SUBMENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                if let Err(err) = hover_app.update(cx, |this, cx| this.hover_inbox_menu(ix, cx)) {
                    log::debug!("inbox menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_inbox_menu(ix, cx)) {
                    log::debug!("inbox menu pick after app drop: {err:#}");
                }
            },
            move |window, cx| {
                let pointer = window.mouse_position();
                let closed = close_app.update(cx, |this, cx| {
                    let in_submenu = this.rail_ui.submenu.as_ref().is_some_and(|s| {
                        submenu_bounds(&this.rail_submenu_entries(s.kind), s.position).contains(&pointer)
                    });
                    if !in_submenu {
                        this.close_rail_menu(cx);
                    }
                });
                if let Err(err) = closed {
                    log::debug!("inbox menu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }

    /// MonoCode `NotificationMuteDatePicker`: starts at the current mute
    /// when one project is muted to a later time, else an hour from now,
    /// rounded up to the minute.
    pub(super) fn open_mute_picker(
        &mut self,
        ids: Vec<String>,
        title: String,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let now = crate::app::now_ms();
        let current = match ids.as_slice() {
            [one] => self
                .settings
                .rail
                .project_notifications
                .get(one)
                .and_then(|p| p.muted_until.flatten())
                .filter(|until| *until > now),
            _ => None,
        };
        let initial = current.unwrap_or(now + HOUR_MS);
        let rounded = (initial + MINUTE_MS - 1) / MINUTE_MS * MINUTE_MS;
        let Some(value) = local_datetime(rounded) else {
            return;
        };
        self.rail_ui.mute_picker = Some(MutePicker {
            ids,
            title,
            position: at,
            value,
            error: None,
        });
        cx.notify();
    }

    fn submit_mute_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.rail_ui.mute_picker.as_mut() else {
            return;
        };
        let now = crate::app::now_ms();
        let error = match local_ms(picker.value) {
            None => Some("Choose a valid date and time."),
            Some(until) if until <= now => Some("Choose a date and time in the future."),
            Some(_) => None,
        };
        if let Some(error) = error {
            picker.error = Some(error.into());
            cx.notify();
            return;
        }
        let (ids, until) = (picker.ids.clone(), local_ms(picker.value));
        self.update_rail_prefs(|prefs| prefs.with_mute(&ids, Some(until), now), cx);
        self.close_rail_menu(cx);
    }

    /// `Popover width={280} className="space-y-1 p-3"`.
    pub(super) fn render_mute_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.rail_ui.mute_picker.as_ref()?;
        let fg = cx.theme().colors.fg;
        let today = local_datetime(crate::app::now_ms()).map(|d| d.date());
        let date_time = DateTimePicker::new("rail-mute-until", Some(picker.value))
            .size(ControlSize::Sm)
            .on_change(app_callback_with(cx, |this, value: DateTime, cx| {
                if let Some(picker) = this.rail_ui.mute_picker.as_mut() {
                    picker.value = value;
                    picker.error = None;
                    cx.notify();
                }
            }));
        let date_time = match today {
            Some(today) => date_time.today(today),
            None => date_time,
        };
        let frame = crate::ui::sidebar_popovers::popover_frame(cx)
            .id("rail-mute-picker")
            .occlude()
            .w(px(PICKER_WIDTH))
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.close_rail_menu(cx);
            }))
            .child(
                // `truncate px-1 text-xs font-medium text-content/85`
                div()
                    .truncate()
                    .px_1()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.opacity(0.85))
                    .child(picker.title.clone()),
            )
            .child(
                // `mb-3 px-1 text-[11px] text-content/45`
                div()
                    .mb_3()
                    .px_1()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .child("Mute all notifications until"),
            )
            .child(date_time)
            .children(picker.error.clone().map(|err| {
                // `mt-3 px-1 text-xs text-red-400`
                div()
                    .mt_3()
                    .px_1()
                    .text_size(px(12.0))
                    .text_color(rgb(RED_400))
                    .child(err)
            }))
            .child(
                // `mt-3 flex justify-between gap-2 border-t border-stroke pt-2.5`
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_t_1()
                    .border_color(fg.opacity(0.07))
                    .pt(px(10.0))
                    .child(
                        Button::new("rail-mute-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_rail_menu(cx);
                            })),
                    )
                    .child(
                        Button::new("rail-mute-submit", "Mute until then")
                            .primary()
                            .size(ControlSize::Sm)
                            .disabled(picker.ids.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.submit_mute_picker(cx))),
                    ),
            );
        Some(
            deferred(anchored().position(picker.position).snap_to_window().child(frame))
                .with_priority(3)
                .into_any_element(),
        )
    }
}
