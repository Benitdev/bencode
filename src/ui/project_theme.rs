//! A project's Theme colors (MonoCode GPUI `project_theme.rs`), from its
//! rail menu: Appearance's accent, hue, saturation and dark-mode lightness
//! for this project alone, shown live while it is open. The first change
//! gives the project colours of its own, starting from Appearance's; Use
//! app colors gives them back. Stored in `settings.json`
//! (`projectThemes`, `ui/rail/model.rs`), applied by
//! `app/preferences.rs` (`follow_project_colors`).

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::overlays::Dialog;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::settings_appearance::ColorScope;
use crate::ui::settings_parts::{SettingsGroup, SettingsRow};

impl BenCodeApp {
    /// Full-window: in `BenCodeApp::render`'s `dialogs` list.
    pub(crate) fn render_project_theme_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let path = self.rail_ui.theme_dialog.clone()?;
        let name = self.rail_project_label(&path);
        let own = self.settings.rail.theme(&path).is_some();
        let scope = ColorScope::Project(path.clone());
        let open = crate::app::same_project_path(&path, &self.current_cwd);
        let detail = match (own, open) {
            (true, _) => format!("{name} shows these colors instead of Appearance's."),
            (false, true) => format!("{name} uses Appearance's colors. A change here is its own."),
            (false, false) => format!(
                "{name} uses Appearance's colors. A change here is its own, shown once it is open."
            ),
        };
        let colors = SettingsGroup::new("Colors")
            .description("The accent colors the send button and your message bubbles.")
            .row(SettingsRow::new("Accent").control(self.render_accent_picker(&scope, cx)))
            .rows(self.render_color_rows(&scope, cx));
        let close = app_callback(cx, |this, cx| {
            this.rail_ui.theme_dialog = None;
            this.accent_picker = None;
            cx.notify();
        });
        let reset = app_callback(cx, move |this, cx| {
            this.accent_picker = None;
            this.set_project_theme(&path, None, cx);
        });
        Some(
            Dialog::new("project-theme", "Theme Colors", close)
                .detail(detail)
                .child(div().p_4().w_full().child(colors))
                .action(move |_| {
                    Button::new("project-theme-reset", "Use app colors")
                        .variant(ButtonVariant::Ghost)
                        .disabled(!own)
                        .on_click(move |_, window, cx| reset(window, cx))
                })
                .action(|close| {
                    Button::new("project-theme-done", "Done")
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .into_any_element(),
        )
    }
}
