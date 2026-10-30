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

/// Hours and minutes as two scrolling columns (minutes alone when `hours`
/// is off). A click sets that part.
fn time_columns(
    id: &ElementId,
    time: Time,
    (step, hours): (u8, bool),
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
    let show_hours = hours;
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
            .debug_selector(move || format!("time-{key}"))
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
        .when(show_hours, |el| {
            el.child(column("hours", &hours).children(hour_cells))
        })
        .child(
            column("minutes", &minutes)
                .when(show_hours, |el| el.border_l_1().border_color(colors.border))
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
                quiet: false,
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
                    (step, true),
                    Rc::new(move |time, window, cx| on_time(day.to_datetime(time), window, cx)),
                    window,
                    cx,
                );
                let out = close.clone();
                float(
                    id.clone(),
                    anchor,
                    CALENDAR_ROWS,
                    surface((id.clone(), "popup"), window, cx)
                        // Wheels and hovers here are the popup's, not the page's.
                        .occlude()
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

/// Digits typed into an open time picker, read as they come: "9" is
/// 09:00, "930" 09:30, "0930" 09:30; with minutes alone, "5" is :05.
fn typed_time(digits: &str, hours: bool, time: Time) -> Option<Time> {
    let number = |text: &str| text.parse::<i8>().ok();
    let (hour, minute) = match (hours, digits.len()) {
        (false, 1 | 2) => (time.hour(), number(digits)?),
        (true, 1 | 2) => (number(digits)?, 0),
        (true, 3) => (number(&digits[..1])?, number(&digits[1..])?),
        (true, 4) => (number(&digits[..2])?, number(&digits[2..])?),
        _ => return None,
    };
    Time::new(hour, minute, 0, 0).ok()
}

/// What an open time picker's keys have typed so far.
#[derive(Default)]
struct Typed(String);

/// A time of day: hour and minute columns to click, and keys for the
/// keyboard. Up and Down step a minute (with Shift, an hour); digits type
/// the time ("0930"); Enter or Escape closes.
#[derive(IntoElement)]
pub struct TimePicker {
    id: ElementId,
    value: Time,
    hours: bool,
    size: ControlSize,
    on_change: Option<OnTime>,
}

impl TimePicker {
    pub fn new(id: impl Into<ElementId>, value: Time) -> Self {
        Self {
            id: id.into(),
            value,
            hours: true,
            size: ControlSize::default(),
            on_change: None,
        }
    }

    /// Minutes past the hour only (":05"), for something hourly.
    pub fn minutes_only(mut self) -> Self {
        self.hours = false;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(Time, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for TimePicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (id, value, hours) = (self.id.clone(), self.value, self.hours);
        let state = dropdown(&id, window, cx);
        let typed = window.use_keyed_state((id.clone(), "typed"), cx, |_, _| Typed::default());
        if !state.read(cx).open && !typed.read(cx).0.is_empty() {
            typed.update(cx, |typed, _| typed.0.clear());
        }
        let shown = match typed.read(cx).0.as_str() {
            "" if hours => show_time(value),
            "" => format!(":{:02}", value.minute()),
            digits if hours => format!("{digits}…"),
            digits => format!(":{digits}…"),
        };
        let commit: OnTime = {
            let (id, on_change) = (id.clone(), self.on_change);
            Rc::new(move |time, window, cx| {
                log::info!("time picker {id:?}: {time}");
                if let Some(on_change) = &on_change {
                    on_change(time, window, cx);
                }
            })
        };
        picker_field(
            self.id,
            Face {
                icon: IconName::Clock,
                shown: Some(shown.into()),
                placeholder: SharedString::default(),
                size: self.size,
                disabled: false,
                quiet: false,
            },
            move |close, anchor, window, cx| {
                let inner = state.read(cx).inner.clone();
                let columns = time_columns(&id, value, (1, hours), commit.clone(), window, cx);
                let (out, keys) = (close.clone(), close);
                float(
                    id.clone(),
                    anchor,
                    CALENDAR_ROWS,
                    surface((id.clone(), "popup"), window, cx)
                        // Wheels and hovers here are the popup's, not the page's.
                        .occlude()
                        .track_focus(&inner)
                        .on_mouse_down_out(move |_, window, cx| out(window, cx))
                        .on_key_down(move |event, window, cx| {
                            let key = event.keystroke.key.as_str();
                            let shift = event.keystroke.modifiers.shift;
                            let step = |minutes: i64| {
                                value.wrapping_add(jiff::SignedDuration::from_mins(minutes))
                            };
                            match key {
                                "up" | "down" => {
                                    let by = if shift && hours { 60 } else { 1 };
                                    let by = if key == "up" { by } else { -by };
                                    commit(step(by), window, cx);
                                }
                                "enter" | "escape" => keys(window, cx),
                                "backspace" => typed.update(cx, |typed, cx| {
                                    typed.0.pop();
                                    cx.notify();
                                }),
                                digit
                                    if digit.len() == 1
                                        && digit.chars().all(|c| c.is_ascii_digit()) =>
                                {
                                    let mut digits = typed.read(cx).0.clone();
                                    digits.push_str(digit);
                                    if let Some(time) = typed_time(&digits, hours, value) {
                                        commit(time, window, cx);
                                    }
                                    // Complete: the next digit starts over.
                                    if digits.len() >= if hours { 4 } else { 2 } {
                                        digits.clear();
                                    }
                                    typed.update(cx, |typed, cx| {
                                        typed.0 = digits;
                                        cx.notify();
                                    });
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        })
                        .child(columns),
                    window,
                    cx,
                )
            },
            window,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::time;

    use super::typed_time;

    #[test]
    fn typed_digits_read_as_a_time() {
        let now = time(14, 20, 0, 0);
        assert_eq!(typed_time("9", true, now), Some(time(9, 0, 0, 0)));
        assert_eq!(typed_time("17", true, now), Some(time(17, 0, 0, 0)));
        assert_eq!(typed_time("930", true, now), Some(time(9, 30, 0, 0)));
        assert_eq!(typed_time("0905", true, now), Some(time(9, 5, 0, 0)));
        assert_eq!(typed_time("2575", true, now), None);
        assert_eq!(typed_time("7", false, now), Some(time(14, 7, 0, 0)));
        assert_eq!(typed_time("45", false, now), Some(time(14, 45, 0, 0)));
        assert_eq!(typed_time("75", false, now), None);
    }
}
