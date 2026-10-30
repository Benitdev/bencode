//! Summoning a bot from anywhere (BenCode's own): a system-wide shortcut,
//! ⌥Space unless Settings › Bots picks another or none, opens the floating
//! chat over whatever app is in front, its composer ready, on the bot the
//! float last showed (else the one last opened). Pressed again while that
//! chat has the keyboard, it hides it. Saves the switch to BenCode and the
//! copy and paste of what to ask about.

use gpui::{AnyWindowHandle, Context};

use crate::app::BenCodeApp;
use crate::hotkey;

/// A shortcut Settings › Bots offers.
#[derive(Debug, PartialEq, Eq)]
pub struct SummonKey {
    /// How `settings.json` names it (`summonKey`).
    pub id: &'static str,
    pub label: &'static str,
    code: u32,
    modifiers: u32,
}

/// The shortcuts offered, the default first. ⌃Space is left out: macOS
/// switches input sources with it.
pub const SUMMON_KEYS: [SummonKey; 3] = [
    SummonKey {
        id: "alt-space",
        label: "⌥Space",
        code: hotkey::SPACE,
        modifiers: hotkey::OPTION,
    },
    SummonKey {
        id: "ctrl-alt-space",
        label: "⌃⌥Space",
        code: hotkey::SPACE,
        modifiers: hotkey::CONTROL | hotkey::OPTION,
    },
    SummonKey {
        id: "shift-alt-space",
        label: "⇧⌥Space",
        code: hotkey::SPACE,
        modifiers: hotkey::SHIFT | hotkey::OPTION,
    },
];

/// `summonKey` when the user wants no shortcut.
pub const SUMMON_OFF: &str = "off";

/// The shortcut a saved `summonKey` names: the default when unset or
/// unknown, none when off.
pub fn summon_key(saved: Option<&str>) -> Option<&'static SummonKey> {
    match saved {
        Some(SUMMON_OFF) => None,
        Some(id) => SUMMON_KEYS
            .iter()
            .find(|key| key.id == id)
            .or(SUMMON_KEYS.first()),
        None => SUMMON_KEYS.first(),
    }
}

#[derive(Default)]
pub struct SummonState {
    /// The window the floating chat belongs to, once listening.
    main: Option<AnyWindowHandle>,
    /// Why the shortcut is not working: another app holds it.
    pub error: Option<String>,
}

impl BenCodeApp {
    /// Listens for the shortcut and registers the saved one. Once, at
    /// launch.
    pub fn listen_for_summon(&mut self, main: AnyWindowHandle, cx: &mut Context<Self>) {
        let Some(mut presses) = hotkey::listen() else {
            return;
        };
        self.summon.main = Some(main);
        self.register_summon_key();
        cx.spawn(async move |this, cx| {
            while presses.recv().await.is_some() {
                if this.update(cx, |this, cx| this.summon_bot(cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// The saved shortcut registered, or none while bots are off.
    pub(crate) fn register_summon_key(&mut self) {
        if self.summon.main.is_none() {
            return;
        }
        let key = summon_key(self.settings.summon_key.as_deref()).filter(|_| self.bots_enabled());
        self.summon.error = match key {
            Some(key) => hotkey::register(key.code, key.modifiers)
                .err()
                .map(|status| {
                    log::warn!("could not register {}: OSStatus {status}", key.label);
                    format!("{} is taken by another app. Pick another.", key.label)
                }),
            None => {
                hotkey::unregister();
                None
            }
        };
    }

    /// The shortcut was pressed: the floating chat hides if it has the
    /// keyboard, else opens over the app in front.
    fn summon_bot(&mut self, cx: &mut Context<Self>) {
        let Some(main) = self.summon.main else {
            return;
        };
        if !self.bots_enabled() {
            return;
        }
        let last = self.bots.last_bot.clone();
        let roster = self.settings.bots.clone();
        let float = self.bot_float_window();
        let app = cx.entity();
        // The float's window is updated after this update, as opening it is.
        cx.defer(move |cx| {
            let shown = float.and_then(|float| {
                float
                    .update(cx, |float, window, cx| {
                        if window.is_window_active() {
                            float.hide(window, cx);
                            return None;
                        }
                        Some(float.bot_id().to_string())
                    })
                    .ok()
            });
            let target = match shown {
                // It had the keyboard and hid.
                Some(None) => return,
                Some(Some(id)) if roster.iter().any(|bot| bot.id == id) => Some(id),
                _ => crate::app::bots::toggle_target(last.as_deref(), &roster),
            };
            match target {
                Some(id) => app.update(cx, |this, cx| this.open_bot_float_over(&id, main, cx)),
                // No bot yet: BenCode, where the rail invites making one.
                None => {
                    crate::ui::native_window::activate_app(cx);
                    if let Err(err) = main.update(cx, |_, window, _| window.activate_window()) {
                        log::debug!("summoned with BenCode's window gone: {err:#}");
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_key_names_a_shortcut_or_none() {
        assert_eq!(summon_key(None).map(|k| k.id), Some("alt-space"));
        assert_eq!(
            summon_key(Some("ctrl-alt-space")).map(|k| k.label),
            Some("⌃⌥Space")
        );
        assert_eq!(summon_key(Some("gone")).map(|k| k.id), Some("alt-space"));
        assert_eq!(summon_key(Some(SUMMON_OFF)), None);
    }
}
