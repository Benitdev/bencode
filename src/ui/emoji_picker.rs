//! The emoji a picker offers (MonoCode `shared/ui/EmojiPicker.tsx`, which
//! lists Emojibase through `frimousse`): the same nine groups, each with
//! its everyday emoji rather than the whole set, so nothing is bundled or
//! loaded. There is no search and no skin tone, which need that data.
//!
//! [`EmojiPicker`] is the popover itself (`EmojiPickerPopover`), for any
//! view: the bar of group faces, then the group's emoji eight to a row.
//! The Mono composer in BenCode's window (`ui/composer/mono.rs`, MonoCode
//! `MonoComposer` with `controls`) shows it; the floating chat's composer
//! has none, as MonoCode's has none.

use std::time::{Duration, Instant};

use bencode_ui::theme::ActiveTheme;
use gpui::{
    Context, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// A click this soon after a press closed the picker is that press's.
const REOPEN_GUARD: Duration = Duration::from_millis(400);

/// MonoCode `COLUMNS`.
pub const COLUMNS: usize = 8;
/// How many picked emoji the Recent group keeps, newest first.
pub const RECENT_MAX: usize = COLUMNS * 2;

pub struct EmojiGroup {
    pub label: &'static str,
    /// MonoCode `CATEGORY_ICONS`: the face on the bar that shows the group.
    pub icon: &'static str,
    pub emoji: &'static [&'static str],
}

pub const GROUPS: [EmojiGroup; 9] = [
    EmojiGroup {
        label: "Smileys & emotion",
        icon: "😀",
        emoji: &[
            "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "😉", "😊", "😇", "🥰", "😍",
            "🤩", "😘", "😋", "😛", "😜", "🤪", "🤗", "🤭", "🤫", "🤔", "🫡", "😐", "😑", "😶",
            "😏", "😒", "🙄", "😬", "😌", "😔", "😪", "😴", "😷", "🤒", "🤕", "🥵", "🥶", "🥴",
            "😵", "🤯", "🤠", "🥳", "😎", "🤓", "🧐", "😕", "😟", "🙁", "😮", "😯", "😲", "😳",
            "🥺", "😢", "😭", "😱", "😤", "😡", "🤬", "💀", "💩", "🤡", "👻", "👽", "🤖", "❤️",
            "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "💔", "💯", "💥",
        ],
    },
    EmojiGroup {
        label: "People & body",
        icon: "👋",
        emoji: &[
            "👋",
            "🤚",
            "🖐️",
            "✋",
            "🖖",
            "👌",
            "🤌",
            "🤏",
            "✌️",
            "🤞",
            "🤟",
            "🤘",
            "🤙",
            "👈",
            "👉",
            "👆",
            "👇",
            "☝️",
            "👍",
            "👎",
            "✊",
            "👊",
            "🤛",
            "🤜",
            "👏",
            "🙌",
            "🫶",
            "👐",
            "🤲",
            "🤝",
            "🙏",
            "✍️",
            "💪",
            "🧠",
            "👀",
            "👁️",
            "👂",
            "👃",
            "👄",
            "🫂",
            "🙋",
            "🤷",
            "🤦",
            "🙇",
            "💁",
            "🙅",
            "🙆",
            "🧑‍💻",
        ],
    },
    EmojiGroup {
        label: "Animals & nature",
        icon: "🐻",
        emoji: &[
            "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸",
            "🐵", "🐔", "🐧", "🐦", "🦆", "🦉", "🐺", "🐴", "🦄", "🐝", "🦋", "🐌", "🐞", "🐢",
            "🐍", "🐙", "🦀", "🐠", "🐬", "🐳", "🦖", "🌵", "🌲", "🌴", "🌱", "🍀", "🌸", "🌻",
            "🌙", "⭐", "🌈", "🔥", "💧", "⚡",
        ],
    },
    EmojiGroup {
        label: "Food & drink",
        icon: "🍔",
        emoji: &[
            "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🫐", "🍒", "🍑", "🥭", "🍍",
            "🥥", "🥑", "🍅", "🥕", "🌽", "🥔", "🍞", "🥐", "🧀", "🍳", "🥓", "🍔", "🍟", "🍕",
            "🌭", "🌮", "🍜", "🍣", "🍙", "🍚", "🍰", "🎂", "🍪", "🍫", "🍩", "🍿", "☕", "🍵",
            "🧋", "🍺", "🍻", "🥂", "🍷", "🥤",
        ],
    },
    EmojiGroup {
        label: "Travel & places",
        icon: "🚗",
        emoji: &[
            "🚗", "🚕", "🚌", "🚎", "🏎️", "🚓", "🚑", "🚒", "🚚", "🚜", "🛵", "🚲", "🚂", "🚆",
            "🚇", "✈️", "🚀", "🛸", "🚁", "⛵", "🚢", "⚓", "⛽", "🚦", "🗺️", "🏠", "🏢", "🏥",
            "🏦", "🏫", "⛰️", "🏝️", "🏖️", "🌋", "🌍", "🌎", "🌏", "🌅", "🌃", "🎡",
        ],
    },
    EmojiGroup {
        label: "Activities",
        icon: "⚽",
        emoji: &[
            "⚽", "🏀", "🏈", "⚾", "🎾", "🏐", "🏉", "🎱", "🏓", "🏸", "🥊", "🥋", "⛳", "🎣",
            "🎿", "🏂", "🏋️", "🚴", "🏆", "🥇", "🥈", "🥉", "🏅", "🎖️", "🎯", "🎮", "🕹️", "🎲",
            "🧩", "♟️", "🎨", "🎬", "🎤", "🎧", "🎸", "🎹", "🥁", "🎉", "🎊", "🎁",
        ],
    },
    EmojiGroup {
        label: "Objects",
        icon: "💡",
        emoji: &[
            "💡", "🔦", "💻", "⌨️", "🖥️", "🖨️", "🖱️", "📱", "☎️", "📷", "🎥", "📺", "⏰", "⌛",
            "🔋", "🔌", "🧲", "🔧", "🔨", "🛠️", "⚙️", "🧰", "🧪", "🔬", "🔭", "📡", "💊", "🔑",
            "🔒", "🔓", "📦", "📫", "✉️", "📝", "📄", "📊", "📈", "📉", "📅", "📌", "📎", "✂️",
            "🗑️", "📚", "🔖", "💰", "💳", "🧾",
        ],
    },
    EmojiGroup {
        label: "Symbols",
        icon: "🔣",
        emoji: &[
            "✅", "☑️", "✔️", "❌", "❎", "❗", "❓", "⚠️", "🚫", "⛔", "💤", "♻️", "🔁", "🔄",
            "➕", "➖", "➗", "✖️", "♾️", "💲", "™️", "©️", "®️", "🔴", "🟠", "🟡", "🟢", "🔵",
            "🟣", "⚫", "⚪", "🟥", "🟩", "🟦", "⬆️", "⬇️", "⬅️", "➡️", "↩️", "🔔", "🔕", "🔊",
            "🔇", "💬", "💭", "🆗", "🆕", "🆘",
        ],
    },
    EmojiGroup {
        label: "Flags",
        icon: "🏳️",
        emoji: &[
            "🏁",
            "🚩",
            "🏳️",
            "🏴",
            "🏳️‍🌈",
            "🏴‍☠️",
            "🇻🇳",
            "🇺🇸",
            "🇬🇧",
            "🇯🇵",
            "🇰🇷",
            "🇨🇳",
            "🇫🇷",
            "🇩🇪",
            "🇪🇸",
            "🇮🇹",
            "🇨🇦",
            "🇦🇺",
            "🇧🇷",
            "🇮🇳",
            "🇸🇬",
            "🇹🇭",
            "🇮🇩",
            "🇺🇦",
        ],
    },
];

/// MonoCode `rememberEmoji`: `emoji` at the front of the recent ones.
pub fn remember(recent: &mut Vec<String>, emoji: &str) {
    recent.retain(|kept| kept != emoji);
    recent.insert(0, emoji.to_string());
    recent.truncate(RECENT_MAX);
}

/// The picker's state, kept by the view that shows it.
#[derive(Default)]
pub struct EmojiPicker {
    /// The group shown while it is open (`None` shows Recent).
    open: Option<Option<usize>>,
    /// The emoji picked this launch, newest first.
    recent: Vec<String>,
    /// When a press outside it last closed it: a press on its own button
    /// does that too, and the click that follows must not reopen it.
    dismissed: Option<Instant>,
}

impl EmojiPicker {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Its button: opens it, Recent first once there are any, or closes it.
    pub fn toggle(&mut self) {
        let just_dismissed = self
            .dismissed
            .take()
            .is_some_and(|at| at.elapsed() < REOPEN_GUARD);
        if just_dismissed {
            return;
        }
        self.open = match self.open {
            Some(_) => None,
            None => Some(self.recent.is_empty().then_some(0)),
        };
    }

    /// Closes it; whether it was open.
    pub fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    /// MonoCode `insertEmoji`'s bookkeeping: remembered, and the picker shut.
    pub fn picked(&mut self, emoji: &str) {
        remember(&mut self.recent, emoji);
        self.open = None;
    }

    /// The popover while it is open, for the caller to place. `state`
    /// finds the picker on the view; `on_pick` inserts the emoji.
    pub fn render<V: 'static>(
        &self,
        id: &str,
        state: fn(&mut V) -> &mut EmojiPicker,
        on_pick: impl Fn(&mut V, &str, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &Context<V>,
    ) -> Option<Stateful<gpui::Div>> {
        let group = self.open?;
        let theme = cx.theme();
        let (fg, bg) = (theme.colors.fg, theme.colors.bg);
        let dark = theme.is_dark();
        let (label, emoji): (&str, Vec<&str>) = match group {
            None => ("Recent", self.recent.iter().map(String::as_str).collect()),
            Some(ix) => {
                let group = GROUPS.get(ix)?;
                (group.label, group.emoji.to_vec())
            }
        };
        let tab = |tab_id: SharedString, face: &str, target: Option<usize>, cx: &Context<V>| {
            let active = group == target;
            div()
                .id(tab_id)
                .flex()
                .flex_1()
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(15.0))
                .when(active, |el| el.bg(fg.fill(Fill::Strong)))
                .when(!active, |el| el.opacity(0.6).hover(|s| s.opacity(1.0)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    state(view).open = Some(target);
                    cx.notify();
                }))
                .child(face.to_string())
        };
        let mut bar = div().flex().gap(px(2.0)).px_1().pb_1();
        if !self.recent.is_empty() {
            bar = bar.child(tab(format!("{id}-recent").into(), "🕘", None, cx));
        }
        for (ix, group) in GROUPS.iter().enumerate() {
            bar = bar.child(tab(
                format!("{id}-group-{ix}").into(),
                group.icon,
                Some(ix),
                cx,
            ));
        }
        let cells = emoji.into_iter().enumerate().map(|(ix, emoji)| {
            let picked = emoji.to_string();
            let on_pick = on_pick.clone();
            div()
                .id(SharedString::from(format!("{id}-cell-{ix}")))
                .flex()
                .flex_none()
                .size_8()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(20.0))
                .hover(move |s| s.bg(fg.fill(Fill::Strong)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    state(view).picked(&picked);
                    on_pick(view, &picked, window, cx);
                    cx.notify();
                }))
                .child(emoji.to_string())
        });
        let grid = div()
            .id(SharedString::from(format!("{id}-grid")))
            .h(px(192.0))
            .overflow_y_scroll()
            .child(
                div()
                    .px_1()
                    .pt_1()
                    .pb_1()
                    .text_size(Text::Sm.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.ink(Ink::Muted))
                    .child(label.to_string()),
            )
            .child(div().flex().flex_wrap().children(cells));
        Some(
            div()
                .id(SharedString::from(id.to_string()))
                .occlude()
                // Eight cells and the frame's padding.
                .w(px(COLUMNS as f32 * 32.0 + 18.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(fg.fill(Fill::Strong))
                .bg(if dark {
                    bg.blend(fg.fill(Fill::Hover))
                } else {
                    bg
                })
                .shadow_lg()
                .p_2()
                .on_mouse_down_out(cx.listener(move |view, _, _, cx| {
                    let picker = state(view);
                    if picker.close() {
                        picker.dismissed = Some(Instant::now());
                        cx.notify();
                    }
                }))
                .child(bar)
                .child(grid),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_group_lists_an_emoji_twice() {
        for group in &GROUPS {
            let mut seen = std::collections::HashSet::new();
            for emoji in group.emoji {
                assert!(seen.insert(emoji), "{emoji} twice in {}", group.label);
            }
            assert!(!group.emoji.is_empty());
        }
    }

    #[test]
    fn the_newest_pick_leads_the_recent_ones() {
        let mut recent = vec!["👍".to_string(), "🎉".to_string()];
        remember(&mut recent, "🎉");
        assert_eq!(recent, ["🎉", "👍"]);
        for ix in 0..RECENT_MAX + 4 {
            remember(&mut recent, GROUPS[0].emoji[ix]);
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], GROUPS[0].emoji[RECENT_MAX + 3]);
    }
}
