use std::rc::Rc;

use gpui::{
    App, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, div, point, prelude::*,
};
use jiff::civil::{Date, DateTime, Time};

use super::{
    super::options::{Run, float, surface},
    picker::{CALENDAR_ROWS, Face, dropdown, picker_field, popped},
    show_date, show_time, zoned_now,
};
use crate::{
    primitives::IconName,
    theme::{ActiveTheme, ControlSize, Radius, TextSize},
};

type OnTime = Rc<dyn Fn(Time, &mut Window, &mut App)>;
type OnDateTime = Rc<dyn Fn(DateTime, &mut Window, &mut App)>;

/// Scroll handles of the two columns, and the time they last scrolled to.
#[derive(Default)]
struct Columns {
    hours: ScrollHandle,
    minutes: ScrollHandle,
    shown: Option<Time>,
}

/// Hours and minutes as two scrolling columns. A click sets that part.
fn time_columns(
    id: &ElementId,
    time: Time,
    step: u8,
    on_pick: OnTime,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let columns = window.use_keyed_state((id.clone(), "columns"), cx, |_, _| Columns::default());
    let row = cx.theme().control_height(ControlSize::Sm);
    if columns.read(cx).shown != Some(time) {
        let row = row.to_pixels(window.rem_size());
        let above = |ix: usize| point(Pixels::ZERO, -(row * ix.saturating_sub(2) as f32));
        columns.update(cx, |columns, _| {
            columns.hours.set_offset(above(time.hour() as usize));
            columns
                .minutes
                .set_offset(above(time.minute() as usize / step as usize));
            columns.shown = Some(time);
        });
    }
    let (hours, minutes) = {
        let columns = columns.read(cx);
        (columns.hours.clone(), columns.minutes.clone())
    };
    let theme = cx.theme();
    let colors = &theme.colors;
    let cell = |key: &'static str, ix: usize, label: String, on: bool, pick: Run| {
        div()
            .id((key, ix))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(row)
            .px_3()
            .rounded(theme.radius(Radius::Md))
            .cursor_pointer()
            .map(|cell| {
                if on {
                    cell.bg(colors.accent).text_color(colors.on_accent)
                } else {
                    cell.text_color(colors.fg)
                        .hover(|style| style.bg(colors.hover))
                }
            })
            .on_click(move |_, window, cx| pick(window, cx))
            .child(label)
    };
    let column = |key: &'static str, handle: &ScrollHandle| {
        div()
            .id((id.clone(), key))
            .track_scroll(handle)
            .overflow_y_scroll()
            .max_h(theme.list_max_height())
            .p_1()
            .flex()
            .flex_col()
    };
    let hour_cells = (0..24).map(|hour| {
        let pick = on_pick.clone();
        cell(
            "hour",
            hour as usize,
            format!("{hour:02}"),
            hour == time.hour(),
            Rc::new(move |window, cx| {
                pick(
                    time.with().hour(hour).build().expect("hours run 0 to 23"),
                    window,
                    cx,
                )
            }),
        )
    });
    let minute_cells = (0..60).step_by(step as usize).map(|minute| {
        let pick = on_pick.clone();
        cell(
            "minute",
            minute as usize,
            format!("{minute:02}"),
            minute == time.minute(),
            Rc::new(move |window, cx| {
                pick(
                    time.with()
                        .minute(minute)
                        .build()
                        .expect("minutes run 0 to 59"),
                    window,
                    cx,
                )
            }),
        )
    });
    div()
        .flex()
        .text_size(theme.text_size(TextSize::Sm))
        .child(column("hours", &hours).children(hour_cells))
        .child(
            column("minutes", &minutes)
                .border_l_1()
                .border_color(colors.border)
                .children(minute_cells),
        )
}

/// A day and a time: a calendar beside hour and minute columns.
#[derive(IntoElement)]
pub struct DateTimePicker {
    id: ElementId,
    value: Option<DateTime>,
    step: u8,
    placeholder: SharedString,
    today: Option<Date>,
    size: ControlSize,
    on_change: Option<OnDateTime>,
}

impl DateTimePicker {
    pub fn new(id: impl Into<ElementId>, value: Option<DateTime>) -> Self {
        Self {
            id: id.into(),
            value,
            step: 5,
            placeholder: SharedString::from("Pick a date and time"),
            today: None,
            size: ControlSize::default(),
            on_change: None,
        }
    }

    pub fn today(mut self, date: Date) -> Self {
        self.today = Some(date);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(DateTime, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for DateTimePicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (id, step, value, today) = (self.id.clone(), self.step, self.value, self.today);
        let state = dropdown(&id, window, cx);
        let commit: OnDateTime = {
            let (id, on_change) = (id.clone(), self.on_change);
            Rc::new(move |at, window, cx| {
                log::info!("date time picker {id:?}: {at}");
                if let Some(on_change) = &on_change {
                    on_change(at, window, cx);
                }
            })
        };
        picker_field(
            self.id,
            Face {
                icon: IconName::CalendarDays,
                shown: value
                    .map(|at| format!("{} {}", show_date(at.date()), show_time(at.time())).into()),
                placeholder: self.placeholder,
                size: self.size,
                disabled: false,
            },
            move |close, anchor, window, cx| {
                let day = value.map_or_else(
                    || today.unwrap_or_else(|| zoned_now().date()),
                    |at| at.date(),
                );
                let time = value.map_or(Time::midnight(), |at| at.time());
                let mut calendar = popped(&id, &state, (None, None, today), cx);
                if let Some(value) = value {
                    calendar = calendar.selected(value.date());
                }
                let (on_day, on_time) = (commit.clone(), commit);
                let columns = time_columns(
                    &id,
                    time,
                    step,
                    Rc::new(move |time, window, cx| on_time(day.to_datetime(time), window, cx)),
                    window,
                    cx,
                );
                let out = close.clone();
                float(
                    id.clone(),
                    anchor,
                    CALENDAR_ROWS,
                    surface((id.clone(), "popup"), cx)
                        .flex()
                        .on_mouse_down_out(move |_, window, cx| out(window, cx))
                        .child(calendar.on_escape(close).on_pick(move |date, window, cx| {
                            on_day(date.to_datetime(time), window, cx)
                        }))
                        .child(
                            div()
                                .border_l_1()
                                .border_color(cx.theme().colors.border)
                                .child(columns),
                        ),
                    window,
                    cx,
                )
            },
            window,
            cx,
        )
    }
}
