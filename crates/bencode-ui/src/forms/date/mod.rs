mod calendar;
mod picker;
mod time;
use jiff::{
    Timestamp, Zoned,
    civil::{Date, Time},
};

use crate::typography::format::system_zone;

pub use calendar::Calendar;
pub use picker::DatePicker;
pub use time::{DateTimePicker, TimePicker};
/// The system's clock in its own zone, UTC when that zone is unknown.
pub(crate) fn zoned_now() -> Zoned {
    Timestamp::now().to_zoned(system_zone("date pickers"))
}

/// A day as "Sep 25, 2026".
pub(crate) fn show_date(date: Date) -> String {
    date.strftime("%b %-d, %Y").to_string()
}

/// A time as "09:30".
pub(crate) fn show_time(time: Time) -> String {
    time.strftime("%H:%M").to_string()
}
