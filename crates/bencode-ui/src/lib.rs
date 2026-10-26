mod assets;

pub mod buttons;
pub mod canvas;
pub mod data_display;
pub mod documents;
pub mod editor;
pub mod feedback;
pub mod files;
pub mod forms;
pub mod i18n;
pub mod layout;
pub mod lists;
pub mod menus;
pub mod motion;
pub mod navigation;
pub mod overlays;
pub mod primitives;
pub mod settings;
pub mod shell;
pub mod terminal;
pub mod theme;
pub mod typography;

pub use assets::Assets;

use gpui::{App, KeyBinding};

use crate::primitives::{FOCUS_CONTEXT, FocusNext, FocusPrev};

/// Checks the assets, loads the theme, binds Tab and text keys. Call once, first.
pub fn init(cx: &mut App) -> anyhow::Result<()> {
    let missing = assets::missing(cx.asset_source().as_ref());
    if let Some(first) = missing.first() {
        anyhow::bail!(
            "bencode-ui: the asset source lacks {} of its files, first {first}; pass `bencode_ui::Assets`, or `Assets::before` your own, to `Application::with_assets`",
            missing.len()
        );
    }
    log::info!("assets: the app's source loads all of bencode-ui's files");
    setup(cx);
    Ok(())
}

/// `init` without assets or fonts, for app tests that render.
#[cfg(any(test, feature = "test-support"))]
pub fn init_for_tests(cx: &mut App) {
    setup(cx);
}

/// The theme and key bindings.
fn setup(cx: &mut App) {
    theme::Theme::init(cx);
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, Some(FOCUS_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrev, Some(FOCUS_CONTEXT)),
        // Out of a field that keeps Tab, such as a document.
        KeyBinding::new("f6", FocusNext, Some(FOCUS_CONTEXT)),
        KeyBinding::new("shift-f6", FocusPrev, Some(FOCUS_CONTEXT)),
    ]);
    forms::bind_keys(cx);
    editor::bind_keys(cx);
    #[cfg(not(target_family = "wasm"))]
    terminal::bind_keys(cx);
    documents::bind_keys(cx);
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    #[should_panic(expected = "leaked handles")]
    fn a_leaked_entity_fails_its_test(cx: &mut TestAppContext) {
        std::mem::forget(cx.new(|_| ()));
    }
}
