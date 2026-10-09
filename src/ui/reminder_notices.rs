//! MonoCode `ReminderNotices`: the due reminders, top right, each with
//! Open session, Snooze (the "Remind me" presets) and Dismiss.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, SharedString, Styled, canvas, div, prelude::*,
    relative, rgb,
};

use crate::app::BenCodeApp;
use crate::app::reminders::format_reminder_time;
use crate::ui::scale::px;
use crate::ui::sidebar_menus::SidebarMenuKind;

/// MonoCode: `min(320px, 100vw - 24px)` wide.
const PANEL_WIDTH: f32 = 320.0;
/// MonoCode: the Snooze menu sits `4px` under its button.
const SNOOZE_MENU_GAP: f32 = 4.0;
/// Tailwind preflight's `line-height: 1.5` (GPUI defaults to ~1.618).
const PREFLIGHT_LEADING: f32 = 1.5;

/// MonoCode `max-h-[min(320px,50vh)]` on the list.
const LIST_MAX_HEIGHT: f32 = 320.0;
/// MonoCode `text-amber-400` on the clock.
const AMBER_400: u32 = 0xfbbf24;

impl BenCodeApp {
    pub(super) fn render_reminder_notices(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let due = self.due_reminders();
        if due.is_empty() && self.reminder_error.is_none() && self.reminder_failure.is_none() {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        // `border-stroke` / `divide-stroke`: content at 7%.
        let stroke = fg.opacity(0.07);
        // MonoCode's row buttons: `rounded-md px-2 py-1` at the row's
        // `text-[11px]`, each with its own ink and fills.
        let button = |id: SharedString, label: &'static str, ink: f32, fill: f32, hover: f32| {
            div()
                .id(id)
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .cursor_pointer()
                .text_color(fg.opacity(ink))
                .when(fill > 0.0, |el| el.bg(fg.opacity(fill)))
                .hover(move |s| s.bg(fg.opacity(hover)))
                .child(label)
        };
        let count = due.len();
        let items = due.iter().enumerate().map(|(ix, reminder)| {
            let (sid, due_at) = (reminder.session_id.clone(), reminder.due_at);
            let title = crate::app::session_list::display_title(&reminder.title, &reminder.harness);
            let project = crate::ui::inbox_view::project_name(&reminder.cwd);
            let (open_id, title_id, snooze_id, dismiss_id) =
                (sid.clone(), sid.clone(), sid.clone(), sid.clone());
            let snooze_anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
            // `article px-3 py-2.5`, `divide-y divide-stroke` between them.
            div()
                .flex()
                .flex_col()
                .px_3()
                .py(px(10.0))
                .when(ix > 0, |el| el.border_t_1().border_color(stroke))
                .child(
                    div()
                        .id(SharedString::from(format!("due-title-{sid}")))
                        .flex()
                        .flex_col()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_reminder(&title_id, due_at, cx)
                        }))
                        // `block truncate text-[13px] font-medium hover:underline`
                        .child(
                            div()
                                .id(SharedString::from(format!("due-title-text-{sid}")))
                                .truncate()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .hover(|s| s.underline())
                                .child(title),
                        )
                        // `mt-1 block truncate text-[11px] text-content/50`
                        .child(
                            div()
                                .mt_1()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(format!("{project} · {}", format_reminder_time(due_at))),
                        ),
                )
                // `mt-2 flex items-center gap-1.5 text-[11px]`
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .child(
                            // `bg-content/10 hover:bg-content/15`
                            button(
                                SharedString::from(format!("due-open-{sid}")),
                                "Open session",
                                1.0,
                                0.1,
                                0.15,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.open_reminder(&open_id, due_at, cx),
                            )),
                        )
                        .child(
                            // `text-content/65 hover:bg-content/10`
                            // MonoCode opens it at `rect.left, rect.bottom + 4`.
                            button(
                                SharedString::from(format!("due-snooze-{sid}")),
                                "Snooze",
                                0.65,
                                0.0,
                                0.1,
                            )
                            .relative()
                            .child({
                                let anchor = snooze_anchor.clone();
                                canvas(
                                    move |bounds, _, _| anchor.set(Some(bounds)),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full()
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.close_sidebar_menu(cx);
                                    let kind = SidebarMenuKind::Remind {
                                        ids: vec![snooze_id.clone()],
                                    };
                                    let at = snooze_anchor.get().map_or(event.position, |b| {
                                        Point::new(b.left(), b.bottom() + px(SNOOZE_MENU_GAP))
                                    });
                                    this.open_sidebar_menu(kind, at, cx);
                                }),
                            ),
                        )
                        .child(
                            // `ml-auto text-content/50 hover:bg-content/10`
                            button(
                                SharedString::from(format!("due-dismiss-{sid}")),
                                "Dismiss",
                                0.5,
                                0.0,
                                0.1,
                            )
                            .ml_auto()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.cancel_reminders(&[dismiss_id.clone()], Some(due_at), cx)
                                },
                            )),
                        ),
                )
        });
        // MonoCode: `rounded-xl border border-content/15 bg-background-base/95
        // text-content shadow-xl backdrop-blur-xl` (GPUI cannot blur what
        // is behind, so the 95% fill stands alone).
        let panel = div()
            .id("due-reminders")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(PANEL_WIDTH))
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .border_1()
            .border_color(fg.opacity(0.15))
            .bg(colors.bg.opacity(0.95))
            .text_color(fg)
            .line_height(relative(PREFLIGHT_LEADING))
            .shadow_xl()
            .overflow_hidden()
            // `flex items-center gap-2 border-b border-stroke px-3 py-2.5`
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(stroke)
                    .child(
                        Icon::new(IconName::Clock)
                            .size(IconSize::Sm)
                            .color(rgb(AMBER_400)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Due reminders"),
                    )
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(count.to_string()),
                        )
                    }),
            )
            // BenCode's own: a snooze or dismiss that failed to save.
            .when_some(self.reminder_failure.clone(), |el, failure| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .text_size(px(12.0))
                        .text_color(colors.danger)
                        .child(div().flex_1().child(failure))
                        .child(div().text_size(px(11.0)).child(
                            button("due-failure-ok".into(), "OK", 1.0, 0.1, 0.15).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.reminder_failure = None;
                                    cx.notify();
                                }),
                            ),
                        )),
                )
            })
            // `px-3 py-2 text-[12px] text-content/70`, "Retry" an underlined
            // inline button.
            .when(self.reminder_error.is_some(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.7))
                        .child("Couldn’t load reminders.")
                        .child(
                            div()
                                .id("due-retry")
                                .cursor_pointer()
                                .underline()
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_reminders(cx)))
                                .child("Retry"),
                        ),
                )
            })
            .child(
                div()
                    .id("due-reminders-list")
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(items),
            );
        // Placed by `render_corner_notices`, above the harness updates.
        Some(panel.into_any_element())
    }
}
