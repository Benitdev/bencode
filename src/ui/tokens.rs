//! BenCode's design tokens: the few levels views draw with, so a look is
//! changed in one place. Ink is text and icons over the theme's `fg`, Fill
//! the washes behind rows and tiles, `Text` the type sizes, and `Status` the
//! one set of colours a session's or task's state is shown in, wherever it
//! is shown (the sidebar's cards, the rail, a Bot's board and launches).
//!
//! `fg.ink(Ink::Muted)` reads as "muted text"; `fg.opacity(0.45)` does not,
//! and drifts. Pixel sizes still go through `scale::px`.

use gpui::{Hsla, Pixels, rgb};

/// Text and icons over `fg`, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// Titles in rows and cards.
    Strong,
    /// Body text a step below a title.
    Body,
    /// Secondary text: a label beside a value, an idle control.
    Secondary,
    /// Muted text: a row's second line, hints, idle icons.
    Muted,
    /// Barely there: counts beside a title, disabled marks.
    Faint,
}

impl Ink {
    pub const fn alpha(self) -> f32 {
        match self {
            Self::Strong => 0.85,
            Self::Body => 0.7,
            Self::Secondary => 0.55,
            Self::Muted => 0.45,
            Self::Faint => 0.3,
        }
    }
}

/// Washes of `fg` behind things, faintest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Faint,
    /// A row under the pointer; a tile behind an icon.
    Hover,
    /// Hairlines between areas.
    Line,
    /// The selected row or tab.
    Selected,
    /// A pressed control, a strong tile.
    Strong,
    Bold,
}

impl Fill {
    pub const fn alpha(self) -> f32 {
        match self {
            Self::Faint => 0.03,
            Self::Hover => 0.05,
            Self::Line => 0.07,
            Self::Selected => 0.08,
            Self::Strong => 0.1,
            Self::Bold => 0.15,
        }
    }
}

/// `fg.ink(..)` and `fg.fill(..)`.
pub trait Tone {
    fn ink(self, ink: Ink) -> Hsla;
    fn fill(self, fill: Fill) -> Hsla;
}

impl Tone for Hsla {
    fn ink(self, ink: Ink) -> Hsla {
        self.opacity(ink.alpha())
    }

    fn fill(self, fill: Fill) -> Hsla {
        self.opacity(fill.alpha())
    }
}

/// Type sizes, before the interface scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// Badges and counts.
    Xs,
    /// Second lines, captions, section titles.
    Sm,
    /// Rows, menus, most controls.
    Base,
    /// Headers and the chat's smaller text.
    Md,
    /// Prose.
    Lg,
    /// A name being edited, a dialog's headline.
    Xl,
    /// A page's or an item's title.
    Title,
    /// The one headline of an empty view (a new thread's greeting).
    Display,
}

impl Text {
    pub const fn size(self) -> f32 {
        match self {
            Self::Xs => 10.0,
            Self::Sm => 11.0,
            Self::Base => 12.0,
            Self::Md => 13.0,
            Self::Lg => 14.0,
            Self::Xl => 15.0,
            Self::Title => 20.0,
            Self::Display => 26.0,
        }
    }

    /// At the interface scale.
    pub fn px(self) -> Pixels {
        crate::ui::scale::px(self.size())
    }
}

/// Tailwind's `amber-400`: something needs the user.
pub const AMBER_400: u32 = 0xfbbf24;
/// Tailwind's `emerald-400`: finished well.
pub const EMERALD_400: u32 = 0x34d399;
/// Tailwind's `red-400`: failed, or an error to fix.
pub const RED_400: u32 = 0xf87171;
/// Tailwind's `sky-400`: new and untracked files.
pub const SKY_400: u32 = 0x38bdf8;
/// Tailwind's `sky-300`: a link under the pointer, on dark.
pub const SKY_300: u32 = 0x7dd3fc;

/// A link in agent text and artifacts: MonoCode's `text-sky-400/90` on
/// dark; the theme's link blue keeps contrast on light.
pub fn link_color(theme: &bencode_ui::theme::Theme) -> Hsla {
    if theme.is_dark() {
        Hsla::from(rgb(SKY_400)).opacity(0.9)
    } else {
        theme.colors.link
    }
}

/// [`link_color`] under the pointer.
pub fn link_hover_color(theme: &bencode_ui::theme::Theme) -> Hsla {
    if theme.is_dark() {
        rgb(SKY_300).into()
    } else {
        theme.colors.link
    }
}

/// Where a session, task or run is, as every view colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Running now (drawn moving: a ring, a pulse).
    Working,
    /// Waiting on the user: an approval, a question, input.
    Attention,
    Done,
    Failed,
    /// Idle, stopped or not started.
    Quiet,
}

impl Status {
    /// Its colour; `fg` is the theme's, for the states drawn in ink.
    pub fn color(self, fg: Hsla) -> Hsla {
        match self {
            Self::Working => fg.ink(Ink::Body),
            Self::Attention => rgb(AMBER_400).into(),
            Self::Done => rgb(EMERALD_400).into(),
            Self::Failed => rgb(RED_400).into(),
            Self::Quiet => fg.ink(Ink::Muted),
        }
    }
}

impl From<crate::app::bot_sessions::LaunchedState> for Status {
    fn from(state: crate::app::bot_sessions::LaunchedState) -> Self {
        use crate::app::bot_sessions::LaunchedState;
        match state {
            LaunchedState::Working => Self::Working,
            LaunchedState::Input => Self::Attention,
            LaunchedState::Done => Self::Done,
            LaunchedState::Failed => Self::Failed,
            LaunchedState::Draft | LaunchedState::Stopped => Self::Quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_step_down() {
        let ink = [
            Ink::Strong,
            Ink::Body,
            Ink::Secondary,
            Ink::Muted,
            Ink::Faint,
        ];
        assert!(ink.windows(2).all(|w| w[0].alpha() > w[1].alpha()));
        let fill = [
            Fill::Faint,
            Fill::Hover,
            Fill::Line,
            Fill::Selected,
            Fill::Strong,
            Fill::Bold,
        ];
        assert!(fill.windows(2).all(|w| w[0].alpha() < w[1].alpha()));
        // Ink never washes out into a fill.
        assert!(Ink::Faint.alpha() > Fill::Bold.alpha());
    }

    #[test]
    fn a_state_has_one_colour_everywhere() {
        use crate::app::bot_sessions::LaunchedState;
        let fg: Hsla = gpui::white();
        assert_eq!(
            Status::from(LaunchedState::Input).color(fg),
            rgb(AMBER_400).into()
        );
        assert_eq!(
            Status::from(LaunchedState::Failed).color(fg),
            rgb(RED_400).into()
        );
        assert_eq!(
            Status::from(LaunchedState::Done).color(fg),
            rgb(EMERALD_400).into()
        );
        assert_eq!(Status::from(LaunchedState::Stopped), Status::Quiet);
    }
}
