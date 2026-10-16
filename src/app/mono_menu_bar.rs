//! MonoCode `useFloatingMono`'s menu bar half: the icon shown while Monos
//! are on and Settings › Monos › Menu bar icon is, its menu kept to the
//! roster (`mono_menu_bar.rs` draws it), and its picks: a Mono opens its
//! floating chat, Quit quits as ⌘Q does.

use gpui::{AnyWindowHandle, Context};

use crate::app::BenCodeApp;
use crate::mono_menu_bar::{self, Entry, Pick};

/// What the menu was last built from, so a redraw rebuilds nothing.
#[derive(Default)]
pub struct MonoMenuBar {
    installed: bool,
    shown: Option<String>,
}

impl BenCodeApp {
    /// Puts the status item up (hidden until `sync_mono_menu_bar` shows it)
    /// and answers its picks. Once, at launch.
    pub fn listen_for_mono_menu_bar(&mut self, main: AnyWindowHandle, cx: &mut Context<Self>) {
        let mark = crate::ui::mascot::MASCOTS[0].rest;
        let Some(mut picks) = mono_menu_bar::install(&mono_menu_bar::icon(&mark)) else {
            return;
        };
        self.mono_menu_bar.installed = true;
        self.sync_mono_menu_bar(cx);
        cx.spawn(async move |this, cx| {
            while let Some(pick) = picks.recv().await {
                let landed = this.update(cx, |this, cx| match pick {
                    Pick::Mono(id) => this.open_mono_float_over(&id, main, cx),
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
    pub fn sync_mono_menu_bar(&mut self, _cx: &mut Context<Self>) {
        if !self.mono_menu_bar.installed {
            return;
        }
        let visible = self.monos_enabled() && !self.settings.mono_menu_bar_off;
        let entries: Vec<Entry> = if visible {
            self.settings
                .monos
                .iter()
                .map(|mono| {
                    let color = gpui::Rgba::from(self.mono_color(mono));
                    let rgb = [color.r, color.g, color.b].map(|c| (c * 255.0).round() as u8);
                    Entry {
                        id: mono.id.clone(),
                        name: mono.display_name(),
                        portrait: mono_menu_bar::portrait(&self.mono_mascot(mono).rest, rgb),
                    }
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
        if key == self.mono_menu_bar.shown {
            return;
        }
        if visible {
            mono_menu_bar::set_menu(&entries);
        }
        mono_menu_bar::set_visible(visible);
        self.mono_menu_bar.shown = key;
    }
}

/// FNV-1a over the portrait's pixels: a new look rebuilds the menu.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
