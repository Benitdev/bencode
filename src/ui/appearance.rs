//! MonoCode Appearance (`features/settings/model/appearance.ts`, the
//! `--theme-*`, `--user-accent-*` and `--color-diff-*` variables of
//! `src/styles/index.css`): the tint every surface takes, the accent of the
//! send button and the user's bubbles, and the diff colours.
//!
//! The palettes themselves are built in `ui/theme.rs`; what Ely's `Palette`
//! has no room for (the user accent, the diff colours) lives in the
//! [`AppearanceTokens`] global for views to read.

use gpui::{App, Global, Hsla, Rgba, rgb};
use serde::{Deserialize, Serialize};

/// MonoCode `THEME_HUE_*`.
pub const HUE_MIN: f32 = 0.0;
pub const HUE_MAX: f32 = 360.0;
pub const HUE_DEFAULT: f32 = 240.0;
/// MonoCode `THEME_SATURATION_*`, in percent.
pub const SATURATION_MIN: f32 = 0.0;
pub const SATURATION_MAX: f32 = 100.0;
pub const SATURATION_DEFAULT: f32 = 0.0;
/// MonoCode `THEME_DARK_LIGHTNESS_*`, in percent.
pub const DARK_LIGHTNESS_MIN: f32 = 0.0;
pub const DARK_LIGHTNESS_MAX: f32 = 30.0;
pub const DARK_LIGHTNESS_DEFAULT: f32 = 9.0;

/// The rem at 100%: GPUI's and Ely's `base_rem`.
pub const REM: f32 = 16.0;

/// MonoCode `UI_SCALE_*`.
pub const UI_SCALE_MIN: f32 = 0.5;
pub const UI_SCALE_MAX: f32 = 2.0;
pub const UI_SCALE_STEP: f32 = 0.1;
pub const UI_SCALE_DEFAULT: f32 = 1.0;

/// MonoCode `normalizeUiScale`: within range, to a tenth.
pub fn normalize_ui_scale(value: f32) -> f32 {
    if !value.is_finite() {
        return UI_SCALE_DEFAULT;
    }
    (value.clamp(UI_SCALE_MIN, UI_SCALE_MAX) * 10.0).round() / 10.0
}

/// MonoCode `UI_SCALE_PERCENTS`: 50, 60, … 200.
pub fn ui_scale_percents() -> impl Iterator<Item = u32> {
    (5..=20).map(|tenth| tenth * 10)
}

/// MonoCode `ACCENT_COLOR_PRESETS`, after "Default" (the text colour).
pub const ACCENT_PRESETS: [(&str, &str); 6] = [
    ("Blue", "#4da3f5"),
    ("Violet", "#8b5cf6"),
    ("Pink", "#ec4899"),
    ("Red", "#ef4444"),
    ("Orange", "#f59e0b"),
    ("Green", "#10b981"),
];

/// Hue, saturation and dark-mode lightness: MonoCode `--theme-hue`,
/// `--theme-saturation`, `--theme-dark-lightness`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeTint {
    pub hue: f32,
    pub saturation: f32,
    pub dark_lightness: f32,
}

impl Default for ThemeTint {
    fn default() -> Self {
        Self {
            hue: HUE_DEFAULT,
            saturation: SATURATION_DEFAULT,
            dark_lightness: DARK_LIGHTNESS_DEFAULT,
        }
    }
}

impl ThemeTint {
    /// MonoCode `loadThemeHue` & co.: whole numbers within their ranges.
    pub fn clamped(self) -> Self {
        Self {
            hue: self.hue.clamp(HUE_MIN, HUE_MAX).round(),
            saturation: self.saturation.clamp(SATURATION_MIN, SATURATION_MAX).round(),
            dark_lightness: self.dark_lightness.clamp(DARK_LIGHTNESS_MIN, DARK_LIGHTNESS_MAX).round(),
        }
    }

    /// `hsl(var(--theme-hue) var(--theme-saturation) lightness)`.
    pub fn color(&self, lightness: f32) -> Hsla {
        hsl(self.hue, self.saturation, lightness)
    }
}

/// MonoCode `DiffPalette`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffPalette {
    #[default]
    Default,
    Colorblind,
    HighContrast,
}

impl DiffPalette {
    pub const ALL: [DiffPalette; 3] = [DiffPalette::Default, DiffPalette::Colorblind, DiffPalette::HighContrast];

    /// MonoCode's id, as stored.
    pub fn key(self) -> &'static str {
        match self {
            DiffPalette::Default => "default",
            DiffPalette::Colorblind => "colorblind",
            DiffPalette::HighContrast => "high-contrast",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|palette| palette.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffPalette::Default => "Default",
            DiffPalette::Colorblind => "Colorblind",
            DiffPalette::HighContrast => "High contrast",
        }
    }
}

/// MonoCode `CHAT_BACKGROUND_OPACITY_*`, as fractions.
pub const CHAT_BACKGROUND_OPACITY_MIN: f32 = 0.05;
pub const CHAT_BACKGROUND_OPACITY_MAX: f32 = 0.65;
pub const CHAT_BACKGROUND_OPACITY_DEFAULT: f32 = 0.24;

/// MonoCode `ChatBackgroundScope`: every conversation, or only empty ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChatBackgroundScope {
    Empty,
    #[default]
    All,
}

impl ChatBackgroundScope {
    pub const ALL: [ChatBackgroundScope; 2] = [ChatBackgroundScope::Empty, ChatBackgroundScope::All];

    pub fn key(self) -> &'static str {
        match self {
            ChatBackgroundScope::Empty => "empty",
            ChatBackgroundScope::All => "all",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            ChatBackgroundScope::Empty => "Empty only",
            ChatBackgroundScope::All => "All sessions",
        }
    }
}

/// MonoCode `NewThreadBackgroundEffect`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundEffect {
    #[default]
    None,
    Dither,
    Ascii,
    Halftone,
    Scanlines,
    GradientBlur,
}

impl BackgroundEffect {
    pub const ALL: [BackgroundEffect; 6] = [
        BackgroundEffect::None,
        BackgroundEffect::Dither,
        BackgroundEffect::Ascii,
        BackgroundEffect::Halftone,
        BackgroundEffect::Scanlines,
        BackgroundEffect::GradientBlur,
    ];

    pub fn key(self) -> &'static str {
        match self {
            BackgroundEffect::None => "none",
            BackgroundEffect::Dither => "dither",
            BackgroundEffect::Ascii => "ascii",
            BackgroundEffect::Halftone => "halftone",
            BackgroundEffect::Scanlines => "scanlines",
            BackgroundEffect::GradientBlur => "gradient-blur",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|effect| effect.key() == key)
    }

    /// MonoCode `NEW_THREAD_BACKGROUND_EFFECT_LABELS`.
    pub fn label(self) -> &'static str {
        match self {
            BackgroundEffect::None => "None",
            BackgroundEffect::Dither => "Dither",
            BackgroundEffect::Ascii => "ASCII",
            BackgroundEffect::Halftone => "Halftone",
            BackgroundEffect::Scanlines => "Scanlines",
            BackgroundEffect::GradientBlur => "Haze",
        }
    }

    /// MonoCode `NEW_THREAD_BACKGROUND_EFFECT_DESCRIPTIONS`.
    pub fn description(self) -> &'static str {
        match self {
            BackgroundEffect::None => "Shows the original artwork.",
            BackgroundEffect::Dither => "Rebuilds the artwork with a dithered color palette.",
            BackgroundEffect::Ascii => "Recreates the artwork with colored characters on black.",
            BackgroundEffect::Halftone => "Recreates the artwork with colored print dots on black.",
            BackgroundEffect::Scanlines => "Adds a pronounced horizontal display-line texture.",
            BackgroundEffect::GradientBlur => "Blurs and fades the artwork into the background below.",
        }
    }

    /// Whether the effect is drawn differently in the light theme.
    pub fn follows_theme(self) -> bool {
        !matches!(self, BackgroundEffect::None | BackgroundEffect::Dither)
    }
}

/// MonoCode `monocode.chatBackground*` and `newThreadBackgroundEffect`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatBackgroundPrefs {
    /// The saved copy of the image; `None` is no background.
    pub path: Option<String>,
    /// Counts each image chosen, so a new file at the same path reloads.
    pub revision: u64,
    pub empty_opacity: f32,
    pub session_opacity: f32,
    pub scope: ChatBackgroundScope,
    pub effect: BackgroundEffect,
}

impl Default for ChatBackgroundPrefs {
    fn default() -> Self {
        Self {
            path: None,
            revision: 0,
            empty_opacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
            session_opacity: CHAT_BACKGROUND_OPACITY_DEFAULT,
            scope: ChatBackgroundScope::default(),
            effect: BackgroundEffect::default(),
        }
    }
}

/// MonoCode `clampChatBackgroundOpacity`: within range, to a whole percent.
pub fn clamp_background_opacity(value: f32) -> f32 {
    if !value.is_finite() {
        return CHAT_BACKGROUND_OPACITY_DEFAULT;
    }
    (value.clamp(CHAT_BACKGROUND_OPACITY_MIN, CHAT_BACKGROUND_OPACITY_MAX) * 100.0).round() / 100.0
}

/// MonoCode `CollapsedProjectRailMode`: what ⌘B leaves of the project rail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CollapsedRailMode {
    /// A column of icons.
    #[default]
    Compact,
    Hidden,
}

impl CollapsedRailMode {
    pub const ALL: [CollapsedRailMode; 2] = [CollapsedRailMode::Compact, CollapsedRailMode::Hidden];

    pub fn key(self) -> &'static str {
        match self {
            CollapsedRailMode::Compact => "compact",
            CollapsedRailMode::Hidden => "hidden",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            CollapsedRailMode::Compact => "Icon rail",
            CollapsedRailMode::Hidden => "Hidden",
        }
    }
}

/// MonoCode `--color-diff-*`: `add` / `del` are the solid marker hues,
/// `*_fg` readable text on the background, `*_bg` / `*_gutter` row tints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColors {
    pub add: Hsla,
    pub add_fg: Hsla,
    pub add_bg: Hsla,
    pub add_gutter: Hsla,
    pub del: Hsla,
    pub del_fg: Hsla,
    pub del_bg: Hsla,
    pub del_gutter: Hsla,
}

impl DiffColors {
    /// `index.css`'s `html.diff-palette-*` and `.theme-light` overrides.
    pub fn new(palette: DiffPalette, dark: bool) -> Self {
        let (add, add_fg, del, del_fg, bg, gutter) = match (palette, dark) {
            (DiffPalette::Default, true) => (0x10b981, 0x6ee7b7, 0xf43f5e, 0xfda4af, 0.15, 0.25),
            (DiffPalette::Default, false) => (0x10b981, 0x047857, 0xf43f5e, 0xbe123c, 0.15, 0.25),
            (DiffPalette::Colorblind, true) => (0x388bfd, 0x79c0ff, 0xdb6d28, 0xffa657, 0.15, 0.25),
            (DiffPalette::Colorblind, false) => (0x0969da, 0x0550ae, 0xbc4c00, 0x953800, 0.15, 0.25),
            (DiffPalette::HighContrast, true) => (0x58a6ff, 0xcae8ff, 0xf0883e, 0xffdfb6, 0.28, 0.45),
            (DiffPalette::HighContrast, false) => (0x0550ae, 0x032563, 0x953800, 0x471700, 0.28, 0.45),
        };
        let (add, del): (Hsla, Hsla) = (rgb(add).into(), rgb(del).into());
        Self {
            add,
            add_fg: rgb(add_fg).into(),
            add_bg: add.opacity(bg),
            add_gutter: add.opacity(gutter),
            del,
            del_fg: rgb(del_fg).into(),
            del_bg: del.opacity(bg),
            del_gutter: del.opacity(gutter),
        }
    }
}

/// MonoCode `--user-accent-color` and `--user-accent-foreground`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserAccent {
    pub color: Hsla,
    /// Black or white, whichever reads on `color`.
    pub foreground: Hsla,
}

impl UserAccent {
    /// A `#rrggbb` colour, as MonoCode stores it; anything else is none.
    pub fn parse(hex: &str) -> Option<Self> {
        let rgba = parse_hex(hex)?;
        Some(Self {
            color: rgba.into(),
            foreground: accent_foreground(rgba),
        })
    }
}

/// The Appearance page's choices the app keeps (Theme, Translucency and
/// their kin live on `BenCodeApp` already).
#[derive(Clone, Debug, PartialEq)]
pub struct AppearancePrefs {
    pub tint: ThemeTint,
    /// MonoCode `monocode.accentColor`: `#rrggbb`, or `None` for Default.
    pub accent_color: Option<String>,
    pub diff_palette: DiffPalette,
    /// MonoCode `monocode.showExcludedFiles`.
    pub show_excluded_files: bool,
    /// MonoCode `monocode.uiScale`: 1 is 100%.
    pub ui_scale: f32,
    pub chat_background: ChatBackgroundPrefs,
    /// MonoCode `monocode.collapsedProjectRailMode`.
    pub collapsed_rail: CollapsedRailMode,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            tint: ThemeTint::default(),
            accent_color: None,
            diff_palette: DiffPalette::Default,
            show_excluded_files: false,
            ui_scale: UI_SCALE_DEFAULT,
            chat_background: ChatBackgroundPrefs::default(),
            collapsed_rail: CollapsedRailMode::default(),
        }
    }
}

impl AppearancePrefs {
    pub fn tokens(&self) -> AppearanceTokens {
        AppearanceTokens {
            diff_palette: self.diff_palette,
            user_accent: self.accent_color.as_deref().and_then(UserAccent::parse),
        }
    }
}

/// What views read beyond Ely's palette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceTokens {
    pub diff_palette: DiffPalette,
    pub user_accent: Option<UserAccent>,
}

impl Default for AppearanceTokens {
    fn default() -> Self {
        Self { diff_palette: DiffPalette::Default, user_accent: None }
    }
}

impl Global for AppearanceTokens {}

impl AppearanceTokens {
    pub fn set(tokens: AppearanceTokens, cx: &mut App) {
        cx.set_global(tokens);
        cx.refresh_windows();
    }

    fn get(cx: &App) -> AppearanceTokens {
        cx.try_global::<AppearanceTokens>().copied().unwrap_or_default()
    }
}

/// The diff colours for the palette chosen and the mode shown.
pub fn diff_colors(cx: &App) -> DiffColors {
    use ely_gpui_component::theme::ActiveTheme;
    DiffColors::new(AppearanceTokens::get(cx).diff_palette, cx.theme().is_dark())
}

/// The accent the user picked; `None` keeps MonoCode's default look.
pub fn user_accent(cx: &App) -> Option<UserAccent> {
    AppearanceTokens::get(cx).user_accent
}

/// MonoCode `isHexColor`: `#rrggbb`.
pub fn parse_hex(hex: &str) -> Option<Rgba> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(rgb)
}

/// MonoCode `accentForeground`: black on light accents, white on dark ones
/// (relative luminance above 0.179).
fn accent_foreground(color: Rgba) -> Hsla {
    let linear = |c: f32| {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b);
    if luminance > 0.179 { gpui::black() } else { gpui::white() }
}

/// CSS `hsl(h s% l%)`.
pub fn hsl(hue: f32, saturation: f32, lightness: f32) -> Hsla {
    gpui::hsla(
        hue.rem_euclid(360.0) / 360.0,
        (saturation / 100.0).clamp(0.0, 1.0),
        (lightness / 100.0).clamp(0.0, 1.0),
        1.0,
    )
}

/// CSS `color-mix(in srgb, a (1 - amount), b amount)`.
pub fn mix(a: Hsla, b: Hsla, amount: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let lerp = |x: f32, y: f32| x + (y - x) * amount;
    Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(color: Hsla) -> u32 {
        let c = Rgba::from(color);
        let byte = |v: f32| (v * 255.0).round() as u32;
        (byte(c.r) << 16) | (byte(c.g) << 8) | byte(c.b)
    }

    #[test]
    fn default_tint_is_monocode_grey() {
        let tint = ThemeTint::default();
        assert_eq!(hex(tint.color(tint.dark_lightness)), 0x171717);
        assert_eq!(hex(tint.color(92.0)), 0xebebeb);
        assert_eq!(hex(tint.color(97.0)), 0xf7f7f7);
    }

    #[test]
    fn tint_clamps_to_whole_numbers_in_range() {
        let tint = ThemeTint { hue: 400.4, saturation: -3.0, dark_lightness: 12.6 }.clamped();
        assert_eq!(tint, ThemeTint { hue: 360.0, saturation: 0.0, dark_lightness: 13.0 });
    }

    #[test]
    fn mix_matches_css_color_mix() {
        let mixed = mix(rgb(0x171717).into(), rgb(0xebebeb).into(), 0.1);
        assert_eq!(hex(mixed), 0x2c2c2c);
    }

    #[test]
    fn accent_foreground_follows_luminance() {
        let accent = |hex: &str| UserAccent::parse(hex).unwrap().foreground;
        assert_eq!(accent("#f59e0b"), gpui::black());
        assert_eq!(accent("#8b5cf6"), gpui::black());
        assert_eq!(accent("#1d4ed8"), gpui::white());
        assert_eq!(accent("#4da3f5"), gpui::black());
        assert!(UserAccent::parse("blue").is_none());
        assert!(UserAccent::parse("#12345").is_none());
    }

    #[test]
    fn diff_palettes_swap_hues_and_strengths() {
        let default = DiffColors::new(DiffPalette::Default, true);
        assert_eq!(hex(default.add_fg), 0x6ee7b7);
        assert_eq!(hex(DiffColors::new(DiffPalette::Default, false).del_fg), 0xbe123c);
        let high = DiffColors::new(DiffPalette::HighContrast, true);
        assert_eq!(hex(high.add), 0x58a6ff);
        assert!((high.add_bg.a - 0.28).abs() < 1e-6);
        assert_eq!(hex(DiffColors::new(DiffPalette::Colorblind, false).del), 0xbc4c00);
    }

    #[test]
    fn background_and_rail_ids_are_monocodes() {
        for effect in BackgroundEffect::ALL {
            assert_eq!(serde_json::to_string(&effect).unwrap(), format!("\"{}\"", effect.key()));
            assert_eq!(BackgroundEffect::from_key(effect.key()), Some(effect));
        }
        assert_eq!(serde_json::to_string(&ChatBackgroundScope::Empty).unwrap(), r#""empty""#);
        assert_eq!(serde_json::to_string(&CollapsedRailMode::Compact).unwrap(), r#""compact""#);
        assert_eq!(clamp_background_opacity(0.9), 0.65);
        assert_eq!(clamp_background_opacity(0.237), 0.24);
    }

    #[test]
    fn ui_scale_snaps_to_tenths_in_range() {
        assert_eq!(normalize_ui_scale(1.04), 1.0);
        assert_eq!(normalize_ui_scale(1.26), 1.3);
        assert_eq!(normalize_ui_scale(9.0), 2.0);
        assert_eq!(normalize_ui_scale(f32::NAN), 1.0);
        assert_eq!(ui_scale_percents().collect::<Vec<_>>().len(), 16);
    }

    #[test]
    fn diff_palette_ids_are_monocodes() {
        for palette in DiffPalette::ALL {
            assert_eq!(serde_json::to_string(&palette).unwrap(), format!("\"{}\"", palette.key()));
            assert_eq!(DiffPalette::from_key(palette.key()), Some(palette));
        }
    }
}
