use gpui::{BoxShadow, Hsla, Pixels, Rems, Size, px, rems, size};

use super::{Mode, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    Compact,
    #[default]
    Standard,
    Comfortable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSize {
    Sm,
    #[default]
    Md,
    Lg,
}

/// How large an avatar is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
    Xxl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    Xs,
    Sm,
    Base,
    Md,
    Lg,
    Xl,
    Xxl,
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radius {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Raised,
    Floating,
    Modal,
}

pub(super) fn px_to_rems(value: f32) -> Rems {
    rems(value / 16.0)
}

impl Theme {
    pub fn control_height(&self, size: ControlSize) -> Rems {
        let base = match size {
            ControlSize::Sm => 24.0,
            ControlSize::Md => 28.0,
            ControlSize::Lg => 32.0,
        };
        let delta = match self.density {
            Density::Compact => -4.0,
            Density::Standard => 0.0,
            Density::Comfortable => 4.0,
        };
        px_to_rems(base + delta)
    }

    pub fn control_padding(&self, size: ControlSize) -> Rems {
        let base = match size {
            ControlSize::Sm => 8.0,
            ControlSize::Md => 12.0,
            ControlSize::Lg => 16.0,
        };
        let delta = match self.density {
            Density::Compact => -2.0,
            Density::Standard => 0.0,
            Density::Comfortable => 2.0,
        };
        px_to_rems(base + delta)
    }

    pub fn avatar_size(&self, size: AvatarSize) -> Rems {
        px_to_rems(match size {
            AvatarSize::Xs => 20.0,
            AvatarSize::Sm => 24.0,
            AvatarSize::Md => 32.0,
            AvatarSize::Lg => 40.0,
            AvatarSize::Xl => 56.0,
        })
    }

    pub fn icon_size(&self, size: IconSize) -> Rems {
        let base = match size {
            IconSize::Xs => 12.0,
            IconSize::Sm => 14.0,
            IconSize::Md => 16.0,
            IconSize::Lg => 20.0,
            IconSize::Xl => 24.0,
            IconSize::Xxl => 32.0,
        };
        px_to_rems(base * self.font_scale)
    }

    pub fn titlebar_height(&self) -> Rems {
        px_to_rems(38.0)
    }

    /// Underline under composing text.
    pub fn underline_thickness(&self) -> Pixels {
        px(1.0)
    }

    /// A border's width, which does not scale: `border_1`.
    pub fn hairline(&self) -> Pixels {
        px(1.0)
    }

    pub fn caret_width(&self) -> Rems {
        px_to_rems(1.5)
    }

    /// Gap between the edge of a screen or window and what floats by it: a mini window, a toast stack.
    pub fn window_margin(&self) -> Pixels {
        px(24.0)
    }

    /// Narrowest and widest window tab.
    pub fn tab_width(&self) -> (Rems, Rems) {
        (px_to_rems(96.0), px_to_rems(200.0))
    }

    /// The line that marks a chosen tab.
    pub fn tab_indicator(&self) -> Rems {
        px_to_rems(2.0)
    }

    /// Width of a dialog's card.
    pub fn dialog_width(&self) -> Rems {
        px_to_rems(440.0)
    }

    /// How far each level of a tree steps in.
    pub fn tree_indent(&self) -> Rems {
        px_to_rems(16.0)
    }

    /// The label column of a description list.
    pub fn label_width(&self) -> Rems {
        px_to_rems(160.0)
    }

    /// Widest a centered block of prose runs, such as an empty state's help.
    pub fn prose_width(&self) -> Rems {
        px_to_rems(320.0)
    }

    /// Narrowest a menu's panel gets.
    pub fn menu_width(&self) -> Rems {
        px_to_rems(200.0)
    }

    /// A palette's card; its list scrolls inside.
    pub fn palette_size(&self) -> Size<Rems> {
        size(px_to_rems(600.0), px_to_rems(400.0))
    }

    /// Hit area of a split or resize handle.
    pub fn handle_hit(&self) -> Rems {
        px_to_rems(6.0)
    }

    /// Gap between the pointer and a tooltip.
    pub fn cursor_offset(&self) -> Rems {
        px_to_rems(18.0)
    }

    /// A checkbox or radio mark.
    pub fn check_size(&self) -> Rems {
        px_to_rems(16.0)
    }

    /// A dot that marks a state, such as unsaved changes.
    pub fn status_dot(&self) -> Rems {
        px_to_rems(6.0)
    }

    /// A switch's track; its thumb fills the height inside a small inset.
    pub fn switch_track(&self) -> Size<Rems> {
        size(px_to_rems(32.0), px_to_rems(18.0))
    }

    pub fn slider_track(&self) -> Rems {
        px_to_rems(4.0)
    }

    pub fn slider_thumb(&self) -> Rems {
        px_to_rems(16.0)
    }

    /// Height of a color picker's saturation and value plane.
    pub fn color_plane(&self) -> Rems {
        px_to_rems(152.0)
    }

    /// Tallest a list grows before it scrolls.
    pub fn list_max_height(&self) -> Rems {
        px_to_rems(280.0)
    }

    pub fn tooltip_max_width(&self) -> Rems {
        px_to_rems(256.0)
    }

    pub fn text_size(&self, size: TextSize) -> Rems {
        let base = match size {
            TextSize::Xs => 11.0,
            TextSize::Sm => 12.0,
            TextSize::Base => 13.0,
            TextSize::Md => 14.0,
            TextSize::Lg => 16.0,
            TextSize::Xl => 20.0,
            TextSize::Xxl => 24.0,
            TextSize::Display => 32.0,
        };
        px_to_rems(base * self.font_scale)
    }

    pub fn radius(&self, size: Radius) -> Rems {
        let base = match size {
            Radius::Sm => 4.0,
            Radius::Md => 6.0,
            Radius::Lg => 8.0,
            Radius::Xl => 12.0,
        };
        px_to_rems(base * self.radius_scale)
    }

    pub fn elevation(&self, level: Elevation) -> Vec<BoxShadow> {
        let dark = self.mode == Mode::Dark;
        let layers: &[(f32, f32, f32)] = match (level, dark) {
            (Elevation::Raised, false) => &[(1.0, 2.0, 0.06)],
            (Elevation::Floating, false) => &[(1.0, 2.0, 0.04), (6.0, 16.0, 0.08)],
            (Elevation::Modal, false) => &[(2.0, 4.0, 0.04), (16.0, 40.0, 0.14)],
            (Elevation::Raised, true) => &[(1.0, 2.0, 0.4)],
            (Elevation::Floating, true) => &[(2.0, 4.0, 0.3), (8.0, 24.0, 0.45)],
            (Elevation::Modal, true) => &[(4.0, 8.0, 0.3), (20.0, 48.0, 0.55)],
        };
        layers
            .iter()
            .map(|&(y, blur, alpha)| shadow(self.colors.shadow.alpha(alpha), y, blur))
            .collect()
    }
}

fn shadow(color: Hsla, y: f32, blur: f32) -> BoxShadow {
    BoxShadow::new(px(0.0), px(y), color).blur_radius(px(blur))
}
