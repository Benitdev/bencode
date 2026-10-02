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
