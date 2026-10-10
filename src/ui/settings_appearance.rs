//! MonoCode Settings › Appearance (`SettingsView.tsx` `AppearancePage`,
//! `AccentColorPicker` and the header's "Restore defaults"): Theme, Color,
//! Translucency and Layout. State and persistence: `app/preferences.rs`;
//! the colours themselves: `ui/appearance.rs` and `ui/theme.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant, SegmentedControl};
use ely_gpui_component::forms::{Choice, ColorPicker, Select, Slider, Switch};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::settings::{Appearance, ThemeSelector};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    Anchor, Context, Hsla, IntoElement, MouseButton, ObjectFit, ParentElement, SharedString,
    Styled, anchored, deferred, div, img, point, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::session_folders::to_hex;
use crate::settings::ThemePreference;
use crate::ui::app_callback::app_callback_with;
use crate::ui::appearance::{
    ACCENT_PRESETS, BackgroundEffect, CHAT_BACKGROUND_OPACITY_MAX, CHAT_BACKGROUND_OPACITY_MIN,
    ChatBackgroundScope, CollapsedRailMode, DARK_LIGHTNESS_MAX, DARK_LIGHTNESS_MIN, DiffPalette,
    HUE_MAX, HUE_MIN, SATURATION_MAX, SATURATION_MIN, ThemeTint, parse_hex, ui_scale_percents,
};
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::icons::ExtraIcon;

use crate::ui::scale::px;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow};

/// MonoCode `ColorPickerPopover`'s `width={248}`.
const ACCENT_PICKER_WIDTH: f32 = 248.0;

impl BenCodeApp {
    /// MonoCode `AppearancePage`.
    pub(crate) fn render_settings_appearance(&self, cx: &Context<Self>) -> SettingsPage {
        SettingsTab::Appearance
            .page()
            .group(self.render_appearance_theme(cx))
            .group(self.render_appearance_color(cx))
            .group(self.render_appearance_translucency(cx))
            .group(self.render_chat_background_card(cx))
            .group(self.render_appearance_layout(cx))
    }

    /// The header's "Restore defaults": `gap-1.5 rounded-md px-2 py-1
    /// text-[12px] text-content/50 hover:bg-content/10 hover:text-content`.
    pub(crate) fn render_restore_appearance(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        div()
            .id("appearance-restore-defaults")
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .rounded(px(6.0))
            .px_2()
            .py_1()
            .text_size(px(12.0))
            .text_color(fg.opacity(0.5))
            .cursor_pointer()
            .hover(|el| el.bg(fg.opacity(0.1)).text_color(fg))
            .child(Icon::new(IconName::RotateCcw).size(IconSize::Sm))
            .child("Restore defaults")
            .on_click(cx.listener(|this, _, _, cx| this.restore_appearance_defaults(cx)))
    }

    fn render_appearance_theme(&self, cx: &Context<Self>) -> impl IntoElement {
        let appearance = match self.theme_preference {
            ThemePreference::Dark => Appearance::Dark,
            ThemePreference::Light => Appearance::Light,
            ThemePreference::System => Appearance::System,
        };
        let theme = ThemeSelector::new("appearance-theme", appearance).on_change(
            app_callback_with(cx, |this, picked: Appearance, cx| {
                let pref = match picked {
                    Appearance::Dark => ThemePreference::Dark,
                    Appearance::Light => ThemePreference::Light,
                    Appearance::System => ThemePreference::System,
                };
                this.set_theme_preference(pref, cx);
            }),
        );
        let mut diff = SegmentedControl::new(
            "appearance-diff-palette",
            self.appearance.diff_palette.key(),
        )
        .size(ControlSize::Sm);
        for palette in DiffPalette::ALL {
            diff = diff.segment(palette.key(), palette.label(), None);
        }
        let diff =
            diff.on_change(cx.listener(
                |this, key: &SharedString, _, cx| match DiffPalette::from_key(key) {
                    Some(palette) => this.set_diff_palette(palette, cx),
                    None => log::warn!("unknown diff palette {key}"),
                },
            ));
        SettingsGroup::new("Theme")
            .description(
                "Dark and light share the same tint, so the color settings below apply to both.",
            )
            .row(
                SettingsRow::new("Theme")
                    .description("System follows the OS appearance.")
                    .control(theme),
            )
            .row(
                SettingsRow::new("Accent color")
                    .description("Used for the composer send button and your message bubbles.")
                    .control(self.render_accent_picker(cx)),
            )
            .row(
                SettingsRow::new("Diff colors")
                    .description(
                        "Colors for added and removed lines. Colorblind and High contrast use \
                         blue and orange instead of green and red; High contrast adds stronger \
                         tints and text.",
                    )
                    .control(diff),
            )
    }

    /// MonoCode `AccentColorPicker`: Default (the text colour), six presets,
    /// and a custom colour whose picker hangs below the row.
    fn render_accent_picker(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let value = self.appearance.accent_color.as_deref();
        let custom: Option<Hsla> = value
            .filter(|hex| ACCENT_PRESETS.iter().all(|(_, preset)| preset != hex))
            .and_then(parse_hex)
            .map(Into::into);
        let picker_open = self.accent_picker_open;
        // `size-3.5 rounded-full`, `ring-2 ring-content/80 ring-offset-1` when picked.
        let swatch = |id: SharedString, color: Option<Hsla>, selected: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .size(px(20.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(if selected { 18.0 } else { 14.0 }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .when(selected, |el| el.border_2().border_color(fg.opacity(0.8)))
                        .child(div().size(px(14.0)).rounded_full().map(|el| match color {
                            Some(color) => el.bg(color),
                            None => el.border_1().border_color(fg.opacity(0.3)),
                        })),
                )
        };
        let presets = std::iter::once(("Default", None))
            .chain(ACCENT_PRESETS.iter().map(|(name, hex)| (*name, Some(*hex))));
        let mut row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .px(px(2.0));
        for (ix, (name, hex)) in presets.enumerate() {
            let color = hex.and_then(parse_hex).map_or(fg, Into::into);
            row = row.child(
                swatch(
                    SharedString::from(format!("accent-color-{ix}")),
                    Some(color),
                    value == hex,
                )
                .tooltip(Tooltip::text(name))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.accent_picker_open = false;
                    this.set_accent_color(hex.map(str::to_string), cx);
                })),
            );
        }
        let row = row.child(
            swatch(
                SharedString::from("accent-color-custom"),
                custom,
                custom.is_some() || picker_open,
            )
            .tooltip(Tooltip::text("Custom color"))
            // On the press, not the click: the picker's own outside-press
            // has closed it by then, and the click would open it again.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.accent_picker_open = !picker_open;
                    cx.notify();
                }),
            )
            .when(custom.is_none(), |el| {
                el.child(
                    div().absolute().child(
                        Icon::new(IconName::Pipette)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.6)),
                    ),
                )
            }),
        );
        let current: Hsla = value
            .or(Some(ACCENT_PRESETS[0].1))
            .and_then(parse_hex)
            .map_or(fg, Into::into);
        // MonoCode `w-48`, the popover `side="bottom" align="end"`.
        div()
            .relative()
            .w(px(192.0))
            .child(row)
            .when(picker_open, |el| {
                let popover = crate::ui::sidebar_popovers::popover_frame(cx)
                    .id("accent-color-picker-popover")
                    .occlude()
                    .w(px(ACCENT_PICKER_WIDTH))
                    .p_2()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.accent_picker_open = false;
                        cx.notify();
                    }))
                    .child(
                        ColorPicker::new("accent-color-picker", current)
                            .opaque()
                            .on_change(app_callback_with(cx, |this, color: Hsla, cx| {
                                this.set_accent_color(Some(to_hex(color)), cx);
                            })),
                    );
                el.child(
                    div().absolute().bottom_0().right_0().child(
                        deferred(
                            anchored()
                                .anchor(Anchor::TopRight)
                                .offset(point(px(0.0), px(6.0)))
                                .snap_to_window()
                                .child(popover),
                        )
                        .with_priority(3),
                    ),
                )
            })
    }

    fn render_appearance_color(&self, cx: &Context<Self>) -> impl IntoElement {
        let tint = self.appearance.tint;
        let light = !cx.theme().is_dark();
        SettingsGroup::new("Color")
            .description(
                "Hue and saturation tint every surface. Lightness only moves the dark theme.",
            )
            .row(
                SettingsRow::new("Hue")
                    .description("Base hue for accents and tinted surfaces.")
                    .control(slider(
                        "appearance-hue",
                        tint.hue,
                        (HUE_MIN, HUE_MAX),
                        format!("{}°", tint.hue),
                        false,
                        cx,
                        move |this, hue, cx| this.set_theme_tint(ThemeTint { hue, ..tint }, cx),
                    )),
            )
            .row(
                SettingsRow::new("Saturation")
                    .description("How strongly the hue tints the interface. Zero keeps it neutral.")
                    .control(slider(
                        "appearance-saturation",
                        tint.saturation,
                        (SATURATION_MIN, SATURATION_MAX),
                        format!("{}%", tint.saturation),
                        false,
                        cx,
                        move |this, saturation, cx| {
                            this.set_theme_tint(ThemeTint { saturation, ..tint }, cx)
                        },
                    )),
            )
            .row(
                SettingsRow::new("Dark-mode lightness")
                    .description(if light {
                        "This only affects dark mode. Your dark-mode value is preserved."
                    } else {
                        "Base brightness of the dark theme. Lower values are darker; zero is true \
                         black."
                    })
                    .control(slider(
                        "appearance-dark-lightness",
                        tint.dark_lightness,
                        (DARK_LIGHTNESS_MIN, DARK_LIGHTNESS_MAX),
                        format!("{}%", tint.dark_lightness),
                        light,
                        cx,
                        move |this, dark_lightness, cx| {
                            this.set_theme_tint(
                                ThemeTint {
                                    dark_lightness,
                                    ..tint
                                },
                                cx,
                            )
                        },
                    )),
            )
    }

    /// MonoCode Appearance › Translucency: how much of the desktop shows
    /// through the glass panes (`ui::glass`).
    fn render_appearance_translucency(&self, cx: &Context<Self>) -> impl IntoElement {
        let disabled = !self.glass(cx).on;
        let description = if !crate::ui::glass::SUPPORTED {
            "Window translucency needs macOS."
        } else if disabled {
            "Light mode always uses an opaque window, so these are off. Your dark-mode values are \
             preserved."
        } else {
            "How much of the desktop shows through BenCode."
        };
        let percent = (self.sidebar_opacity * 100.0).round();
        let opacity = slider(
            "glass-opacity",
            percent,
            (
                crate::ui::glass::OPACITY_MIN * 100.0,
                crate::ui::glass::OPACITY_MAX * 100.0,
            ),
            format!("{percent}%"),
            disabled,
            cx,
            |this, value, cx| this.set_sidebar_opacity(value / 100.0, cx),
        );
        let body = Switch::new("glass-body", self.body_glass)
            .disabled(disabled)
            .on_change(app_callback_with(cx, |this, on, cx| {
                this.set_body_glass(on, cx);
                this.play_cue(crate::sounds::Cue::Switch);
            }));
        SettingsGroup::new("Translucency")
            .description(description)
            .row(
                SettingsRow::new("Sidebar opacity")
                    .description("Applies to the project rail and the other glass panes.")
                    .control(opacity),
            )
            .row(
                SettingsRow::new("Main pane glass")
                    .description(
                        "Extend the translucent treatment to the main pane behind sessions and \
                         editors.",
                    )
                    .control(body),
            )
    }

    /// MonoCode `ChatBackgroundCard`: the preview (or "Choose an image"),
    /// Change / Remove, and with an image its effect, scope and the two
    /// visibilities.
    fn render_chat_background_card(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let prefs = &self.appearance.chat_background;
        let state = &self.chat_background;
        let has_image = prefs.path.is_some();
        let busy = state.busy;
        let empty_percent = (prefs.empty_opacity * 100.0).round();
        let session_percent = (prefs.session_opacity * 100.0).round();
        // `h-36 overflow-hidden rounded-lg border border-content/10`
        let frame = div()
            .id("chat-background-preview")
            .relative()
            .h(px(144.0))
            .w_full()
            .overflow_hidden()
            .rounded(px(8.0))
            .border_1()
            .border_color(fg.opacity(0.1));
        let preview = if has_image {
            frame
                .children(state.image().map(|image| {
                    img(image.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(prefs.empty_opacity)
                }))
                .child(
                    // `absolute bottom-2 left-2 text-[11px] text-content/40`
                    div()
                        .absolute()
                        .bottom_2()
                        .left_2()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.4))
                        .child(format!("Empty chat preview at {empty_percent}%")),
                )
        } else {
            // `flex-col items-center justify-center gap-2 text-content/40
            // hover:bg-content/5 hover:text-content/70 disabled:opacity-40`
            frame
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.4))
                .when(busy, |el| el.opacity(0.4))
                .when(!busy, |el| {
                    el.cursor_pointer()
                        .hover(|el| el.bg(fg.opacity(0.05)).text_color(fg.opacity(0.7)))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_chat_background(cx)))
                })
                .child(if busy {
                    spinning_icon(
                        "chat-background-busy".into(),
                        IconName::LoaderCircle,
                        IconSize::Lg,
                        fg.opacity(0.4),
                    )
                    .into_any_element()
                } else {
                    ExtraIcon::ImagePlus
                        .icon()
                        .size(IconSize::Lg)
                        .color(fg.opacity(0.4))
                        .into_any_element()
                })
                .child("Choose an image")
        };
        let picture = div()
            .p_4()
            .flex()
            .flex_col()
            .child(preview)
            .when(has_image, |el| {
                el.child(
                    div()
                        .mt_3()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("chat-background-change", "Change")
                                .size(ControlSize::Sm)
                                .loading(busy)
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.choose_chat_background(cx)),
                                ),
                        )
                        .child(
                            Button::new("chat-background-remove", "Remove")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Danger)
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.clear_chat_background(cx)),
                                ),
                        ),
                )
            })
            .children(state.error.clone().map(|error| {
                // `mt-2 text-[12px] text-red-400`
                div()
                    .mt_2()
                    .text_size(px(12.0))
                    .text_color(colors.danger)
                    .child(error)
            }));
        let section = SettingsGroup::new("Chat background")
            .description("An image behind your chat panes. It stays on this device.")
            .row(picture);
        if !has_image {
            return section;
        }
        let mut effect = SegmentedControl::new("chat-background-effect", prefs.effect.key())
            .size(ControlSize::Sm);
        for choice in BackgroundEffect::ALL {
            effect = effect.segment(choice.key(), choice.label(), None);
        }
        let effect = effect.on_change(cx.listener(|this, key: &SharedString, _, cx| {
            match BackgroundEffect::from_key(key) {
                Some(effect) => this.set_background_effect(effect, cx),
                None => log::warn!("unknown background effect {key}"),
            }
        }));
        let mut scope =
            SegmentedControl::new("chat-background-scope", prefs.scope.key()).size(ControlSize::Sm);
        for choice in ChatBackgroundScope::ALL {
            scope = scope.segment(choice.key(), choice.label(), None);
        }
        let scope = scope.on_change(cx.listener(|this, key: &SharedString, _, cx| {
            match ChatBackgroundScope::from_key(key) {
                Some(scope) => this.set_chat_background_scope(scope, cx),
                None => log::warn!("unknown background scope {key}"),
            }
        }));
        let range = (
            CHAT_BACKGROUND_OPACITY_MIN * 100.0,
            CHAT_BACKGROUND_OPACITY_MAX * 100.0,
        );
        section
            .row(
                SettingsRow::new("Background effect")
                    .description(prefs.effect.description())
                    .control(effect),
            )
            .row(
                SettingsRow::new("Show on")
                    .description("Empty sessions only, or every conversation.")
                    .control(scope),
            )
            .row(
                SettingsRow::new("Empty chat visibility")
                    .description("Background strength before a chat has messages.")
                    .control(slider(
                        "chat-background-empty-opacity",
                        empty_percent,
                        range,
                        format!("{empty_percent}%"),
                        false,
                        cx,
                        |this, value, cx| this.set_chat_background_opacity(true, value / 100.0, cx),
                    )),
            )
            .row(
                SettingsRow::new("Session visibility")
                    .description("Background strength once the conversation has messages.")
                    .control(slider(
                        "chat-background-session-opacity",
                        session_percent,
                        range,
                        format!("{session_percent}%"),
                        false,
                        cx,
                        |this, value, cx| {
                            this.set_chat_background_opacity(false, value / 100.0, cx)
                        },
                    )),
            )
    }

    fn render_appearance_layout(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut rail = SegmentedControl::new(
            "appearance-collapsed-rail",
            self.appearance.collapsed_rail.key(),
        )
        .size(ControlSize::Sm);
        for mode in CollapsedRailMode::ALL {
            rail = rail.segment(mode.key(), mode.label(), None);
        }
        let rail = rail.on_change(cx.listener(|this, key: &SharedString, _, cx| {
            match CollapsedRailMode::from_key(key) {
                Some(mode) => this.set_collapsed_rail_mode(mode, cx),
                None => log::warn!("unknown collapsed rail mode {key}"),
            }
        }));
        let percent = (self.appearance.ui_scale * 100.0).round() as u32;
        let scale = Select::new(
            "appearance-ui-scale",
            ui_scale_percents()
                .map(|percent| Choice::new(percent.to_string(), format!("{percent}%"))),
        )
        .label("Interface scale")
        .selected(percent.to_string())
        .on_change(cx.listener(
            |this, value: &SharedString, _, cx| match value.parse::<f32>() {
                Ok(percent) => this.set_ui_scale(percent / 100.0, cx),
                Err(err) => log::warn!("interface scale {value}: {err}"),
            },
        ));
        let excluded = Switch::new(
            "appearance-show-excluded",
            self.appearance.show_excluded_files,
        )
        .on_change(app_callback_with(cx, |this, on, cx| {
            this.set_show_excluded_files(on, cx);
            this.play_cue(crate::sounds::Cue::Switch);
        }));
        SettingsGroup::new("Layout")
            .row(
                SettingsRow::new("Collapsed project rail")
                    .description(
                        "Keep project navigation available as a compact icon rail, or hide the \
                         rail completely.",
                    )
                    .control(rail),
            )
            .row(
                SettingsRow::new("Interface scale")
                    .description("Zoom the whole interface. You can also use ⌘=, ⌘- and ⌘0.")
                    .control(div().min_w(px(96.0)).child(scale)),
            )
            .row(
                SettingsRow::new("Show excluded files")
                    .description(
                        "Show files and folders Git excludes, such as build output and \
                         dependencies, in the explorer.",
                    )
                    .control(excluded),
            )
    }
}

/// MonoCode's settings `Slider`: the track, then the value (`display`).
fn slider(
    id: &'static str,
    value: f32,
    (min, max): (f32, f32),
    display: String,
    disabled: bool,
    cx: &Context<BenCodeApp>,
    on_change: impl Fn(&mut BenCodeApp, f32, &mut Context<BenCodeApp>) + 'static,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div().w(px(180.0)).child(
                Slider::new(id, f64::from(value))
                    .range(f64::from(min), f64::from(max))
                    .step(1.0)
                    .disabled(disabled)
                    .on_change(app_callback_with(cx, move |this, value: f64, cx| {
                        on_change(this, value as f32, cx)
                    })),
            ),
        )
        .child(
            div()
                .w(px(36.0))
                .text_size(px(12.0))
                .text_color(cx.theme().colors.fg_muted)
                .child(display),
        )
}
