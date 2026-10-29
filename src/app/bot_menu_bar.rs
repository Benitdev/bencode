//! MonoCode `useFloatingMono`'s menu bar half: the icon shown while Bots
//! are on and Settings › Bots › Menu bar icon is, its menu kept to the
//! roster (`bot_menu_bar.rs` draws it), and its picks: a Bot opens its
//! floating chat, Quit quits as ⌘Q does.

use gpui::{AnyWindowHandle, Context};

use crate::app::BenCodeApp;
use crate::bot_menu_bar::{self, Entry, Pick};

/// What the menu was last built from, so a redraw rebuilds nothing.
#[derive(Default)]
pub struct BotMenuBar {
    installed: bool,
    shown: Option<String>,
    /// The icon carries its dot: a bot needs you.
    alert: bool,
}

impl BenCodeApp {
    /// Puts the status item up (hidden until `sync_bot_menu_bar` shows it)
    /// and answers its picks. Once, at launch.
    pub fn listen_for_bot_menu_bar(&mut self, main: AnyWindowHandle, cx: &mut Context<Self>) {
        let Some(mut picks) = bot_menu_bar::install(&bot_menu_bar::icon(false)) else {
            return;
        };
        self.bot_menu_bar.installed = true;
        self.sync_bot_menu_bar(cx);
        cx.spawn(async move |this, cx| {
            while let Some(pick) = picks.recv().await {
                let landed = this.update(cx, |this, cx| match pick {
                    Pick::Bot(id) => this.open_bot_float_over(&id, main, cx),
                    Pick::Quit => {
                        // Its confirmation shows in BenCode's window.
                        cx.activate(true);
                        this.request_quit(cx);
                    }
                });
                if landed.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// The icon shown or hidden, and its menu rebuilt when the roster it
    /// lists changed.
    pub fn sync_bot_menu_bar(&mut self, _cx: &mut Context<Self>) {
        if !self.bot_menu_bar.installed {
            return;
        }
        let visible = self.bots_enabled() && !self.settings.bot_menu_bar_off;
        let entries: Vec<Entry> = if visible {
            self.settings
                .bots
                .iter()
                .map(|bot| Entry {
                    id: bot.id.clone(),
                    name: bot.display_name(),
                    portrait: bot_menu_bar::portrait(bot.figure(), self.bot_color(bot)),
                })
                .collect()
        } else {
            Vec::new()
        };
        let key = visible.then(|| {
            entries
                .iter()
                .map(|e| format!("{}\u{1}{}\u{1}{:x}", e.id, e.name, fingerprint(&e.portrait)))
                .collect::<Vec<_>>()
                .join("\u{0}")
        });
        if key == self.bot_menu_bar.shown {
            return;
        }
        if visible {
            bot_menu_bar::set_menu(&entries);
        }
        bot_menu_bar::set_visible(visible);
        self.bot_menu_bar.shown = key;
    }
}

impl BenCodeApp {
    /// The icon's dot follows whether any bot needs you; the app's clock
    /// checks once a tick, and the image changes only when that does.
    pub(crate) fn sync_bot_menu_bar_alert(&mut self) {
        if !self.bot_menu_bar.installed {
            return;
        }
        let alert = self.bots_enabled()
            && self
                .settings
                .bots
                .iter()
                .any(|bot| self.bot_state(bot).status == crate::bots::BotStatus::NeedsYou);
        if alert != self.bot_menu_bar.alert {
            self.bot_menu_bar.alert = alert;
            bot_menu_bar::set_icon(&bot_menu_bar::icon(alert));
        }
    }
}

/// FNV-1a over the portrait's pixels: a new look rebuilds the menu.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
