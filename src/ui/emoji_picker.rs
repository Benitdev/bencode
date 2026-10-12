//! The emoji a picker offers (MonoCode `shared/ui/EmojiPicker.tsx`, which
//! lists Emojibase through `frimousse`): the same nine groups, each with
//! its everyday emoji rather than the whole set, so nothing is bundled or
//! loaded. There is no search and no skin tone, which need that data.

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
