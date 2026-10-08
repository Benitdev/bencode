//! Adapts a `BenCodeApp` method to Ely's `Fn(&mut Window, &mut App)` callbacks.

use gpui::{App, Context, Window};

use crate::app::BenCodeApp;

pub fn app_callback(
    cx: &Context<BenCodeApp>,
    f: impl Fn(&mut BenCodeApp, &mut Context<BenCodeApp>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let listener = cx.listener(move |this, _: &(), _, cx| f(this, cx));
    move |window, cx| listener(&(), window, cx)
}

/// `app_callback` for Ely callbacks that hand over a value first, e.g.
/// `Fn(Hsla, &mut Window, &mut App)`.
pub fn app_callback_with<T: 'static>(
    cx: &Context<BenCodeApp>,
    f: impl Fn(&mut BenCodeApp, T, &mut Context<BenCodeApp>) + 'static,
) -> impl Fn(T, &mut Window, &mut App) + 'static {
    let entity = cx.entity().downgrade();
    move |value, _, cx| {
        if let Err(err) = entity.update(cx, |this, cx| f(this, value, cx)) {
            log::debug!("callback after app drop: {err:#}");
        }
    }
}

/// A `Select`'s `on_change` for a `BenCodeApp` method.
pub fn on_value(
    cx: &Context<BenCodeApp>,
    f: impl Fn(&mut BenCodeApp, &str, &mut Context<BenCodeApp>) + 'static,
) -> impl Fn(&gpui::SharedString, &mut Window, &mut App) + 'static {
    let entity = cx.entity().downgrade();
    move |value, _, cx| {
        if let Err(err) = entity.update(cx, |this, cx| f(this, value, cx)) {
            log::debug!("select after app drop: {err:#}");
        }
    }
}
