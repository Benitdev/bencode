use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit,
    ParentElement, RenderOnce, SharedString, Styled, Window, div, prelude::*,
};

use crate::{
    primitives::{Icon, IconName, Image},
    theme::{ActiveTheme, AvatarSize, IconSize, Palette, Radius, TextSize},
};

/// Whether someone is around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Online,
    Away,
    Busy,
    Offline,
}

impl Presence {
    pub fn color(self, colors: &Palette) -> Hsla {
        match self {
            Self::Online => colors.success,
            Self::Away => colors.warning,
            Self::Busy => colors.danger,
            Self::Offline => colors.fg_subtle,
        }
    }
}

/// Up to two capitals from a name's first and last words.
pub(crate) fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = |word: &str| word.chars().next().into_iter().flat_map(char::to_uppercase);
    match words.as_slice() {
        [] => String::new(),
        [only] => first(only).collect(),
        [head, .., tail] => first(head).chain(first(tail)).collect(),
    }
}

/// The chart color a name keeps, so the same person reads the same way everywhere.
fn tint(name: &str, colors: &Palette) -> Hsla {
    let hash = name.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    colors.chart[hash as usize % colors.chart.len()]
}

/// A person or a team at a glance: their picture, or their initials on a tone kept by the name, or an icon. A presence dot can sit on the corner; squares suit teams.
#[derive(IntoElement)]
pub struct Avatar {
    id: ElementId,
    name: SharedString,
    image: Option<ImageSource>,
    icon: Option<IconName>,
    size: AvatarSize,
    square: bool,
    ring: bool,
    ring_color: Option<Hsla>,
    presence: Option<Presence>,
}

impl Avatar {
    /// The name gives the initials and the tone when there is no picture.
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            image: None,
            icon: None,
            size: AvatarSize::default(),
            square: false,
            ring: false,
            ring_color: None,
            presence: None,
        }
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let side = theme.avatar_size(self.size);
        let rounded = |element: gpui::Div| {
            if self.square {
                element.rounded(theme.radius(Radius::Md))
            } else {
                element.rounded_full()
            }
        };
        let tone = tint(&self.name, colors);
        let text = match self.size {
            AvatarSize::Xs | AvatarSize::Sm => TextSize::Xs,
            AvatarSize::Md => TextSize::Sm,
            AvatarSize::Lg => TextSize::Md,
            AvatarSize::Xl => TextSize::Lg,
        };
        let face: AnyElement = match (self.image, self.icon) {
            (Some(source), _) => {
                let mut picture = Image::new((self.id.clone(), "picture"), source)
                    .fit(ObjectFit::Cover)
                    .size_full();
                picture = if self.square {
                    picture.rounded(theme.radius(Radius::Md))
                } else {
                    picture.rounded_full()
                };
                picture.into_any_element()
            }
            (None, Some(icon)) => Icon::new(icon)
                .size(IconSize::Md)
                .color(colors.fg_muted)
                .into_any_element(),
            (None, None) => initials(&self.name).into_any_element(),
        };
        let dot = self.presence.map(|presence| {
            div()
                .absolute()
                .right_0()
                .bottom_0()
                .size(side * 0.28)
                .rounded_full()
                .border_2()
                .border_color(colors.bg)
                .bg(presence.color(colors))
        });
        div()
            .relative()
            .flex_none()
            .size(side)
            .child(
                rounded(div())
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(tone.opacity(0.16))
                    .text_color(tone)
                    .text_size(theme.text_size(text))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(self.ring, |face| {
                        face.border_2()
                            .border_color(self.ring_color.unwrap_or(colors.bg))
                    })
                    .child(face),
            )
            .children(dot)
    }
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn initials_take_the_first_and_last_words() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("Grace Brewster Murray Hopper"), "GH");
        assert_eq!(initials("mia"), "M");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("élodie durand"), "ÉD");
    }
}
