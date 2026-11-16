//! Settings › Shortcuts: BenCode's keys in groups, read only. MonoCode's
//! Keybindings page (`SettingsView.tsx` `KeybindingsPage`) also rebinds
//! them; BenCode's keymap is fixed (`app/commands.rs`), and the keys shown
//! here are looked up in it, so the page cannot drift from what is bound.

use std::sync::OnceLock;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use bencode_ui::typography::KbdCombo;
use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::command_catalog::{Group, groups};
use crate::app::commands::{self as cmd, chord_for};
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow};
use crate::ui::tokens::{Ink, Tone as _};

/// A row with its keys found in the keymap.
struct Shown {
    label: &'static str,
    icon: IconName,
    keys: String,
    note: Option<&'static str>,
}

/// [`groups`] with their keys, looked up once: the keymap does not change
/// while BenCode runs.
fn shown() -> &'static [Group<Shown>] {
    static SHOWN: OnceLock<Vec<Group<Shown>>> = OnceLock::new();
    SHOWN.get_or_init(|| {
        let keymap = cmd::keymap();
        groups()
            .into_iter()
            .map(|group| Group {
                title: group.title,
                description: group.description,
                rows: group
                    .rows
                    .into_iter()
                    .filter_map(|entry| {
                        let Some(keys) = chord_for(&keymap, entry.action.as_ref(), entry.context)
                        else {
                            log::warn!("shortcuts page: nothing binds {:?}", entry.label);
                            return None;
                        };
                        Some(Shown {
                            label: entry.label,
                            icon: entry.icon,
                            keys,
                            note: entry.note,
                        })
                    })
                    .collect(),
            })
            .collect()
    })
}

impl BenCodeApp {
    pub(crate) fn render_settings_shortcuts(&self, cx: &Context<Self>) -> SettingsPage {
        let muted = cx.theme().colors.fg.ink(Ink::Secondary);
        shown()
            .iter()
            .fold(SettingsTab::Shortcuts.page(), |page, group| {
                let card = SettingsGroup::new(group.title).rows(group.rows.iter().map(|row| {
                    let item = SettingsRow::new(row.label)
                        .leading(Icon::new(row.icon).size(IconSize::Sm).color(muted))
                        .control(KbdCombo::new(&row.keys));
                    match row.note {
                        Some(note) => item.description(note),
                        None => item,
                    }
                }));
                page.group(match group.description {
                    Some(text) => card.description(text),
                    None => card,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_shortcut_is_bound() {
        let keymap = cmd::keymap();
        for group in groups() {
            for entry in group.rows {
                assert!(
                    chord_for(&keymap, entry.action.as_ref(), entry.context).is_some(),
                    "{} › {} has no binding",
                    group.title,
                    entry.label
                );
            }
        }
    }
}
