//! MonoCode `ReminderNotices`: the due reminders, top right, each with
//! Open session, Snooze (the "Remind me" presets) and Dismiss.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, SharedString, Styled, canvas, div, prelude::*,
    relative, rgb,
};

use crate::app::BenCodeApp;
use crate::app::reminders::format_reminder_time;
use crate::ui::scale::px;
use crate::ui::sidebar_menus::SidebarMenuKind;
use crate::ui::tokens::AMBER_400;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode: `min(320px, 100vw - 24px)` wide.
const PANEL_WIDTH: f32 = 320.0;
/// MonoCode: the Snooze menu sits `4px` under its button.
const SNOOZE_MENU_GAP: f32 = 4.0;
/// Tailwind preflight's `line-height: 1.5` (GPUI defaults to ~1.618).
const PREFLIGHT_LEADING: f32 = 1.5;

/// MonoCode `max-h-[min(320px,50vh)]` on the list.
const LIST_MAX_HEIGHT: f32 = 320.0;

impl BenCodeApp {
    pub(super) fn render_reminder_notices(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let due = self.due_reminders();
        if due.is_empty() && self.reminder_error.is_none() && self.reminder_failure.is_none() {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        // `border-stroke` / `divide-stroke`: content at 7%.
        let stroke = fg.fill(Fill::Line);
        // The row's actions: the one to take filled, the others quiet.
        let button = |id: SharedString, label: &'static str, filled: bool| {
            Button::new(id, label)
                .size(ControlSize::Sm)
                .variant(if filled {
                    ButtonVariant::Subtle
                } else {
                    ButtonVariant::Ghost
                })
        };
        let count = due.len();
        let items = due.iter().enumerate().map(|(ix, reminder)| {
            let (sid, due_at) = (reminder.session_id.clone(), reminder.due_at);
            let title = crate::app::session_list::display_title(&reminder.title, &reminder.harness);
            let project = crate::ui::inbox_view::project_name(&reminder.cwd);
            let (open_id, title_id, snooze_id, dismiss_id) =
                (sid.clone(), sid.clone(), sid.clone(), sid.clone());
            let snooze_anchor: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
            let canvas_anchor = snooze_anchor.clone();
            // `article px-3 py-2.5`, `divide-y divide-stroke` between them.
            div()
                .px_3()
                .py(px(10.0))
                .when(ix > 0, |el| el.border_t_1().border_color(stroke))
                .child(
                    div()
                        .id(SharedString::from(format!("due-title-{sid}")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_reminder(&title_id, due_at, cx)
                        }))
                        // `block truncate text-[13px] font-medium hover:underline`
                        .child(
                            div()
                                .id(SharedString::from(format!("due-title-text-{sid}")))
                                .truncate()
                                .text_size(Text::Md.px())
                                .font_weight(FontWeight::MEDIUM)
                                .hover(|s| s.underline())
                                .child(title),
                        )
                        // `mt-1 block truncate text-[11px] text-content/50`
                        .child(
                            div()
                                .mt_1()
                                .truncate()
                                .text_size(Text::Sm.px())
                                .text_color(fg.ink(Ink::Secondary))
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
                        .text_size(Text::Sm.px())
                        .child(
                            button(
                                SharedString::from(format!("due-open-{sid}")),
                                "Open session",
                                true,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.open_reminder(&open_id, due_at, cx),
                            )),
                        )
                        .child(
                            // MonoCode opens it at `rect.left, rect.bottom + 4`.
                            div()
                                .relative()
                                .flex_none()
                                .child(
                                    button(
                                        SharedString::from(format!("due-snooze-{sid}")),
                                        "Snooze",
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            this.close_sidebar_menu(cx);
                                            let kind = SidebarMenuKind::Remind {
                                                ids: vec![snooze_id.clone()],
                                            };
                                            let at =
                                                snooze_anchor.get().map_or(event.position(), |b| {
                                                    Point::new(
                                                        b.left(),
                                                        b.bottom() + px(SNOOZE_MENU_GAP),
                                                    )
                                                });
                                            this.open_sidebar_menu(kind, at, cx);
                                        },
                                    )),
                                )
                                .child({
                                    let anchor = canvas_anchor;
                                    canvas(
                                        move |bounds, _, _| anchor.set(Some(bounds)),
                                        |_, _, _, _| {},
                                    )
                                    .absolute()
                                    .size_full()
                                }),
                        )
                        .child(
                            div().ml_auto().child(
                                button(
                                    SharedString::from(format!("due-dismiss-{sid}")),
                                    "Dismiss",
                                    false,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.cancel_reminders(
                                            &[dismiss_id.clone()],
                                            Some(due_at),
                                            cx,
                                        )
                                    },
                                )),
                            ),
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
            .border_color(fg.fill(Fill::Bold))
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
                            .text_size(Text::Base.px())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Due reminders"),
                    )
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .text_size(Text::Sm.px())
                                .text_color(fg.ink(Ink::Secondary))
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
                        .text_size(Text::Base.px())
                        .text_color(colors.danger)
                        .child(div().flex_1().child(failure))
                        .child(div().text_size(Text::Sm.px()).child(
                            button("due-failure-ok".into(), "OK", true).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.reminder_failure = None;
                                    cx.notify();
                                },
                            )),
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
                        .text_size(Text::Base.px())
                        .text_color(fg.ink(Ink::Body))
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
