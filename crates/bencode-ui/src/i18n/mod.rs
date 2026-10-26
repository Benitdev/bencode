mod builtin;
mod locale;
use std::collections::HashMap;

use gpui::{App, Global, SharedString};

pub use locale::{Direction, Locale};

type Catalog = HashMap<&'static str, &'static str>;

/// The locale shown and a catalog of messages per locale; every catalog holds the first one's keys.
pub struct I18n {
    locale: Locale,
    catalogs: Vec<(&'static str, Catalog)>,
}

impl Global for I18n {}

impl I18n {
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// The locale's own message for `key`, nothing when its catalog lacks it.
    pub(crate) fn find(&self, key: &str) -> Option<&'static str> {
        self.catalogs
            .iter()
            .find(|(tag, _)| *tag == self.locale.tag)
            .and_then(|(_, catalog)| catalog.get(key).copied())
    }
}

/// A component's message: the app's catalog first, the built-in copy after. Without an `I18n` global it reads the English builtin.
pub fn text(cx: &App, key: &str, args: &[(&str, &str)]) -> SharedString {
    let (message, direction) = if cx.has_global::<I18n>() {
        let i18n = cx.global::<I18n>();
        let message = i18n
            .find(key)
            .unwrap_or_else(|| builtin::builtin(i18n.locale().tag, key));
        (message, i18n.locale().direction)
    } else {
        (builtin::builtin("en-US", key), Direction::Ltr)
    };
    let text = fill(message, args).unwrap_or_else(|error| panic!("{key:?}: {error}"));
    match direction {
        Direction::Ltr => text.into(),
        Direction::Rtl => format!("\u{200f}{text}").into(),
    }
}

/// `message` with each `{name}` replaced by its argument.
fn fill(message: &str, args: &[(&str, &str)]) -> Result<String, String> {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or("a { without }")?;
        let name = &after[..close];
        let (_, value) = args
            .iter()
            .find(|(arg, _)| *arg == name)
            .ok_or_else(|| format!("no argument {name:?}"))?;
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
