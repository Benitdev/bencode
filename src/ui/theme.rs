use gpui::{Rgba, rgb, rgba};

/// MonoCode's exact design tokens and color palette
pub struct MonoTheme;

impl MonoTheme {
    // Backgrounds
    #[inline] pub fn bg_base() -> Rgba { rgb(0x0e1015) }       // hsl(220, 20%, 7%)
    #[inline] pub fn bg_surface() -> Rgba { rgb(0x13161c) }    // hsl(220, 18%, 9%)
    #[inline] pub fn bg_hover() -> Rgba { rgb(0x1c202a) }      // hsl(220, 16%, 14%)
    #[inline] pub fn bg_active() -> Rgba { rgb(0x222734) }     // hsl(220, 16%, 17%)
    #[inline] pub fn bg_card() -> Rgba { rgb(0x161922) }

    // Separators & Borders
    #[inline] pub fn border_stroke() -> Rgba { rgba(0xffffff10) }   // white 6%
    #[inline] pub fn border_strong() -> Rgba { rgba(0xffffff20) }   // white 12%
    #[inline] pub fn border_accent() -> Rgba { rgba(0x388bfd40) }

    // Accent Colors
    #[inline] pub fn accent() -> Rgba { rgb(0x388bfd) }        // MonoCode Electric Blue hsl(211, 92%, 62%)
    #[inline] pub fn accent_hover() -> Rgba { rgb(0x4f9aff) }
    #[inline] pub fn on_accent() -> Rgba { rgb(0xffffff) }

    #[inline] pub fn skill_gold() -> Rgba { rgb(0xe8c547) }
    #[inline] pub fn mention_cyan() -> Rgba { rgb(0x38bdf8) }

    // Status Colors
    #[inline] pub fn success() -> Rgba { rgb(0x2ea043) }
    #[inline] pub fn success_bg() -> Rgba { rgba(0x2ea04318) }
    #[inline] pub fn warning() -> Rgba { rgb(0xd29922) }
    #[inline] pub fn warning_bg() -> Rgba { rgba(0xd2992218) }
    #[inline] pub fn danger() -> Rgba { rgb(0xf85149) }
    #[inline] pub fn danger_bg() -> Rgba { rgba(0xf8514918) }

    // Foregrounds / Text
    #[inline] pub fn fg_primary() -> Rgba { rgb(0xf1f3f7) }
    #[inline] pub fn fg_base() -> Rgba { rgb(0xf1f3f7) }
    #[inline] pub fn fg_muted() -> Rgba { rgb(0x8b949e) }
    #[inline] pub fn fg_subtle() -> Rgba { rgb(0x565f6d) }

    #[inline] pub fn status_error() -> Rgba { rgb(0xf85149) }
    #[inline] pub fn status_error_bg() -> Rgba { rgba(0xf8514918) }

    // Agent Harness Branding
    #[inline] pub fn claude_orange() -> Rgba { rgb(0xd97706) }
    #[inline] pub fn codex_green() -> Rgba { rgb(0x10b981) }
    #[inline] pub fn antigravity_blue() -> Rgba { rgb(0x3b82f6) }
}
