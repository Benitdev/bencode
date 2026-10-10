//! The composer's model picker (MonoCode `ModelPicker`). The menu lists the
//! model's settings (Fast, Effort, Context, …) and a Model row; hovering a
//! row opens its flyout: the setting's choices, or the models with a
//! provider rail (Favorites first), a search field and stars. ⌘. and a
//! right-click on the chip open the recent-models menu instead.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::ui::scale::px;
use serde_json::{Map, Value};

use super::HARNESS_ORDER;
use super::menus::popover_surface;
use crate::app::BenCodeApp;
use crate::harness::HarnessKind;
use crate::harness::catalog::{self, ModelOption, ModelSetting, SettingKind};
use crate::ui::HarnessIcon;
use crate::ui::composer::ComposerPopover;
use crate::ui::sidebar_popovers::popover_glass;

/// MonoCode `MENU_WIDTH`, `MODEL_MENU_WIDTH`, `SETTING_MENU_WIDTH`.
const MENU_WIDTH: f32 = 250.0;
const MODEL_MENU_WIDTH: f32 = 310.0;
const SETTING_MENU_WIDTH: f32 = 210.0;
/// Flyouts overlap the menu by 4px (MonoCode `SUBMENU_OVERLAP`).
const SUBMENU_OVERLAP: f32 = 4.0;
const ENTRY_HEIGHT: f32 = 36.0;
const OPTION_HEIGHT: f32 = 32.0;
const PROVIDER_TAB_SIZE: f32 = 32.0;
const RECENT_MODELS_SHOWN: usize = 6;

/// The flyout open beside the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submenu {
    /// The setting at this menu row.
    Setting(usize),
    Models,
}

/// MonoCode `ModelPickerTab`: the provider rail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelTab {
    #[default]
    Favorites,
    Harness(HarnessKind),
}

/// MonoCode `MODEL_MENU_HEIGHT`: the provider rail sets the flyout height.
fn model_menu_height() -> f32 {
    let tabs = HARNESS_ORDER.len() as f32;
    (tabs + 1.0) * PROVIDER_TAB_SIZE + tabs * 4.0 + 12.0 + 2.0
}

/// Models of `tab` matching `query` against name and provider.
pub fn filtered_models(tab: ModelTab, favorites: &[String], query: &str) -> Vec<ModelOption> {
    let query = query.trim().to_lowercase();
    let pool: Vec<ModelOption> = match tab {
        ModelTab::Favorites => favorites.iter().filter_map(|k| catalog::find(k)).collect(),
        ModelTab::Harness(kind) => catalog::models_for(kind).to_vec(),
    };
    pool.into_iter()
        .filter(|m| {
            query.is_empty()
                || format!(
                    "{} {} {}",
                    m.label,
                    m.harness.label(),
                    m.provider.as_deref().unwrap_or("")
                )
                .to_lowercase()
                .contains(&query)
        })
        .collect()
}

/// MonoCode `recentMenuModels`: recent choices, the current model always
/// among them, at most six.
pub fn recent_menu_models(recent: &[String], current: &str) -> Vec<ModelOption> {
    let mut models: Vec<_> = recent.iter().filter_map(|k| catalog::find(k)).collect();
    if let Some(current) = catalog::find(current)
        && !models.iter().any(|m| m.key == current.key)
    {
        models.push(current);
    }
    models.truncate(RECENT_MODELS_SHOWN);
    models
}

impl BenCodeApp {
    fn model_query(&self, cx: &Context<Self>) -> String {
        self.model_search_input.read(cx).text().to_string()
    }

    pub(super) fn current_model_key(&self) -> String {
        self.selected_session()
            .map_or(self.selected_model.clone(), |s| s.model.clone())
    }

    /// The thread's settings; with no thread, the last ones chosen
    /// (MonoCode `lastModelSettings`).
    pub(super) fn current_model_values(&self) -> Option<&Map<String, Value>> {
        match self.selected_session() {
            Some(session) => session.model_settings.as_ref(),
            None => Some(&self.last_model_settings),
        }
    }

    fn visible_models(&self, cx: &Context<Self>) -> Vec<ModelOption> {
        // Favorites only offer installed harnesses (MonoCode `visibleHarnesses`).
        let favorites: Vec<String> = self
            .favorite_models
            .iter()
            .filter(|k| catalog::find(k).is_some_and(|m| self.harness_available(m.harness)))
            .cloned()
            .collect();
        filtered_models(
            self.composer_menus.model_tab,
            &favorites,
            &self.model_query(cx),
        )
    }

    /// The CLI was found by the startup probe (unknown harnesses count as
    /// available).
    pub(crate) fn harness_available(&self, kind: HarnessKind) -> bool {
        self.harnesses
            .iter()
            .find(|h| h.id == kind.id())
            .is_none_or(|h| h.available)
    }

    /// MonoCode `harnessUnavailableHint`.
    fn harness_unavailable_hint(&self, kind: HarnessKind) -> String {
        let name = self
            .harnesses
            .iter()
            .find(|h| h.id == kind.id())
            .map_or(kind.label(), |h| h.name);
        format!("{name} not found. Install it, or restart BenCode if it is already installed.")
    }

    /// The rail's provider tabs: installed harnesses only.
    fn rail_harnesses(&self) -> Vec<HarnessKind> {
        HARNESS_ORDER
            .into_iter()
            .filter(|k| self.harness_available(*k))
            .collect()
    }

    /// The highlighted row on the current model, else the top.
    pub(crate) fn highlight_current_model(&mut self, cx: &Context<Self>) {
        let current = self.current_model_key();
        self.model_picker_index = self
            .visible_models(cx)
            .iter()
            .position(|m| m.key == current)
            .unwrap_or(0);
    }

    /// Opens the menu on its first row with the current provider's tab, or
    /// closes it.
    pub fn toggle_model_picker(&mut self, cx: &mut Context<Self>) {
        if self.popover_open(ComposerPopover::Model) {
            self.close_model_picker(cx);
            return;
        }
        self.close_composer_popovers(cx);
        self.composer_popover = Some(ComposerPopover::Model);
        let current = self.current_model_key();
        if let Some(model) = catalog::find(&current) {
            self.refresh_model_catalog(model.harness, cx);
        }
        let menus = &mut self.composer_menus;
        menus.model_entry = 0;
        menus.model_submenu = None;
        // MonoCode `coerceModelPickerTab`: a hidden provider falls back to
        // Favorites.
        let rail = self.rail_harnesses();
        let menus = &mut self.composer_menus;
        menus.model_tab = catalog::find(&current)
            .filter(|m| rail.contains(&m.harness))
            .map_or(ModelTab::Favorites, |m| ModelTab::Harness(m.harness));
        self.model_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.highlight_current_model(cx);
        self.focus_composer_menu(cx);
        cx.notify();
    }

    /// ⌘. or a right-click on the chip: the recent-models menu.
    pub fn toggle_recent_models(&mut self, cx: &mut Context<Self>) {
        if self.composer_menus.recent_open {
            self.close_model_picker(cx);
            return;
        }
        self.close_composer_popovers(cx);
        let current = self.current_model_key();
        self.composer_menus.recent_index = recent_menu_models(&self.recent_models, &current)
            .iter()
            .position(|m| m.key == current)
            .unwrap_or(0);
        self.composer_menus.recent_open = true;
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub fn close_model_picker(&mut self, cx: &mut Context<Self>) {
        self.close_popover(ComposerPopover::Model);
        self.composer_menus.recent_open = false;
        self.composer_menus.model_submenu = None;
        self.refocus_prompt(cx);
        cx.notify();
    }

    fn pick_model(&mut self, key: &str, cx: &mut Context<Self>) {
        if catalog::find(key).is_some_and(|m| !self.harness_available(m.harness)) {
            return;
        }
        self.set_session_model(key, cx);
        self.close_model_picker(cx);
    }

    /// ↑/↓ over the models, stopping at the ends (MonoCode `ModelFlyout`).
    pub fn move_model_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.visible_models(cx).len();
        if len > 0 {
            self.model_picker_index = self
                .model_picker_index
                .saturating_add_signed(delta)
                .min(len - 1);
            cx.notify();
        }
    }

    /// Enter: picks the highlighted model.
    pub fn pick_highlighted_model(&mut self, cx: &mut Context<Self>) {
        let models = self.visible_models(cx);
        if let Some(model) = models.get(self.model_picker_index).or(models.first()) {
            self.pick_model(&model.key, cx);
        }
    }

    fn select_model_tab(&mut self, tab: ModelTab, cx: &mut Context<Self>) {
        self.composer_menus.model_tab = tab;
        if let ModelTab::Harness(kind) = tab {
            self.refresh_model_catalog(kind, cx);
        }
        self.model_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.model_picker_index = 0;
        cx.notify();
    }

    /// MonoCode `onModelSettingsChange`: the focused thread's settings take
    /// `value`, and it is remembered for threads to come.
    fn set_model_setting(&mut self, id: &str, value: &str, cx: &mut Context<Self>) {
        let mut next =
            catalog::merge_settings(&self.current_model_key(), self.current_model_values());
        next.insert(id.to_string(), Value::String(value.to_string()));
        self.save_last_model_settings(&next, false, cx);
        if let Some(session) = self.selected_session_mut() {
            if id == "context" && session.context_window.is_some() {
                session.context_window =
                    Some(catalog::context_window_tokens(&session.model, Some(&next)));
            }
            session.model_settings = Some(next);
            let id = session.id.clone();
            self.persist_session(&id);
        }
        cx.notify();
    }

    fn toggle_model_setting(&mut self, setting: &ModelSetting, cx: &mut Context<Self>) {
        let next = if setting.value(self.current_model_values()) == "true" {
            "false"
        } else {
            "true"
        };
        self.set_model_setting(&setting.id, next, cx);
    }

    /// MonoCode `showEntrySubmenu`: a select or the Model row opens its
    /// flyout on the current value; a toggle has none.
    fn show_entry_submenu(&mut self, entry: usize, cx: &mut Context<Self>) {
        let settings = catalog::picker_settings(&self.current_model_key());
        self.composer_menus.model_submenu = match settings.get(entry) {
            Some(setting) if setting.kind == SettingKind::Select => {
                let value = setting.value(self.current_model_values());
                self.composer_menus.setting_index = setting
                    .options
                    .iter()
                    .position(|(v, _)| *v == value)
                    .unwrap_or(0);
                Some(Submenu::Setting(entry))
            }
            Some(_) => None,
            None => {
                if self.composer_menus.model_submenu != Some(Submenu::Models) {
                    self.highlight_current_model(cx);
                }
                Some(Submenu::Models)
            }
        };
        cx.notify();
    }

    /// MonoCode `onMenuKey` for the model menu.
    pub(super) fn model_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let settings = catalog::picker_settings(&self.current_model_key());
        let entries = settings.len() + 1;
        let submenu = self.composer_menus.model_submenu;
        match (key, submenu) {
            ("down" | "up", Some(Submenu::Models)) => {
                self.move_model_picker(if key == "down" { 1 } else { -1 }, cx);
            }
            ("down" | "up", Some(Submenu::Setting(ix))) => {
                let last = settings[ix].options.len().saturating_sub(1);
                let index = &mut self.composer_menus.setting_index;
                *index = if key == "down" {
                    (*index + 1).min(last)
                } else {
                    index.saturating_sub(1)
                };
            }
            ("down" | "up", None) => {
                let step = if key == "down" { 1 } else { entries - 1 };
                self.composer_menus.model_entry =
                    (self.composer_menus.model_entry + step) % entries;
            }
            ("right", _) => self.show_entry_submenu(self.composer_menus.model_entry, cx),
            ("left", _) => self.composer_menus.model_submenu = None,
            ("enter", Some(Submenu::Models)) => self.pick_highlighted_model(cx),
            ("enter", Some(Submenu::Setting(ix))) => {
                let setting = &settings[ix];
                if let Some((value, _)) = setting.options.get(self.composer_menus.setting_index) {
                    self.set_model_setting(&setting.id, value, cx);
                }
                self.close_model_picker(cx);
            }
            ("enter", None) => match settings.get(self.composer_menus.model_entry) {
                Some(setting) if setting.kind == SettingKind::Toggle => {
                    self.toggle_model_setting(setting, cx)
                }
                _ => self.show_entry_submenu(self.composer_menus.model_entry, cx),
            },
            ("escape", _) => self.close_model_picker(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// The recent-models menu: ↑/↓ wrap, Enter or Space picks.
    pub(super) fn recent_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let models = recent_menu_models(&self.recent_models, &self.current_model_key());
        let len = models.len().max(1);
        let index = &mut self.composer_menus.recent_index;
        match key {
            "down" => *index = (*index + 1) % len,
            "up" => *index = (*index + len - 1) % len,
            "enter" | "space" => {
                if let Some(model) = models.get(*index) {
                    self.pick_model(&model.key, cx);
                }
            }
            "escape" => self.close_model_picker(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// MonoCode's model menu: setting rows, then Model, with the open
    /// flyout beside them.
    pub(super) fn render_model_picker_popover(
        &self,
        current_key: &str,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        let settings = catalog::picker_settings(current_key);
        let values = self.current_model_values();
        let active = self.composer_menus.model_entry;
        let row = |ix: usize, id: SharedString| {
            let hover = colors.fg.opacity(0.05);
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .h(px(ENTRY_HEIGHT))
                .px_2()
                .rounded(px(8.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .text_color(colors.fg)
                .when(ix == active, |el| el.bg(colors.active))
                .when(ix != active, |el| el.hover(move |s| s.bg(hover)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.composer_menus.model_entry = ix;
                        this.show_entry_submenu(ix, cx);
                    }
                }))
        };
        let chevron = || {
            Icon::new(IconName::ChevronRight)
                .size(IconSize::Xs)
                .color(colors.fg.opacity(0.45))
        };
        let setting_rows = settings.iter().enumerate().map(|(ix, setting)| {
            let base = row(
                ix,
                SharedString::from(format!("model-setting-{}", setting.id)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(setting.menu_label().to_string()),
            );
            match setting.kind {
                SettingKind::Toggle => {
                    let on = setting.value(values) == "true";
                    let toggled = setting.clone();
                    base.on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_model_setting(&toggled, cx)),
                    )
                    .child(toggle_switch(on, cx))
                }
                SettingKind::Select => base
                    .on_click(cx.listener(move |this, _, _, cx| this.show_entry_submenu(ix, cx)))
                    .child(
                        div()
                            .max_w(px(112.0))
                            .truncate()
                            .text_color(colors.fg.opacity(0.55))
                            .child(setting.value_label(values).to_string()),
                    )
                    .child(chevron()),
            }
        });
        let model_ix = settings.len();
        let current = catalog::find(current_key);
        let model_row = row(model_ix, "model-row".into())
            .on_click(cx.listener(move |this, _, _, cx| this.show_entry_submenu(model_ix, cx)))
            .child(div().flex_1().min_w_0().child("Model"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .max_w(px(144.0))
                    .text_color(colors.fg.opacity(0.55))
                    .children(current.map(|m| HarnessIcon::new(m.harness.id()).size(px(14.0))))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(catalog::label_for(current_key)),
                    ),
            )
            .child(chevron());
        let menu_height = 2.0 + 8.0 + (settings.len() + 1) as f32 * ENTRY_HEIGHT;
        let flyout = match self.composer_menus.model_submenu {
            Some(Submenu::Setting(ix)) => settings
                .get(ix)
                .map(|setting| self.render_setting_flyout(setting, ix, menu_height, cx)),
            Some(Submenu::Models) => Some(self.render_model_flyout(current_key, cx)),
            None => None,
        };
        div()
            .id("composer-model-popover")
            .track_focus(&self.composer_menus.focus)
            .absolute()
            .bottom(px(32.0))
            .left_0()
            .w(px(MENU_WIDTH))
            .p_1()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .flex()
            .flex_col()
            .children(setting_rows)
            .child(model_row)
            .children(flyout)
    }

    /// A select's choices, beside its row, with a check on the current one.
    fn render_setting_flyout(
        &self,
        setting: &ModelSetting,
        row: usize,
        menu_height: f32,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let current = setting.value(self.current_model_values()).to_string();
        let active = self.composer_menus.setting_index;
        let height = 2.0 + 8.0 + setting.options.len() as f32 * OPTION_HEIGHT;
        // Beside its row, lifted when it would run off the composer's foot.
        let top = (row as f32 * ENTRY_HEIGHT - 1.0).min(menu_height + 40.0 - height);
        let options = setting
            .options
            .iter()
            .enumerate()
            .map(|(ix, (value, label))| {
                let hover = colors.fg.opacity(0.05);
                div()
                    .id(SharedString::from(format!(
                        "setting-opt-{}-{value}",
                        setting.id
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(OPTION_HEIGHT))
                    .px_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .text_color(colors.fg)
                    .when(ix == active, |el| el.bg(colors.active))
                    .when(ix != active, |el| el.hover(move |s| s.bg(hover)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.composer_menus.setting_index != ix {
                            this.composer_menus.setting_index = ix;
                            cx.notify();
                        }
                    }))
                    .on_click({
                        let (id, value) = (setting.id.clone(), value.clone());
                        cx.listener(move |this, _, _, cx| {
                            this.set_model_setting(&id, &value, cx);
                            this.close_model_picker(cx);
                        })
                    })
                    .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                    .when(*value == current, |el| {
                        el.child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Xs)
                                .color(colors.fg.opacity(0.5)),
                        )
                    })
            });
        let flyout = div()
            .id("model-setting-flyout")
            .absolute()
            .left(px(MENU_WIDTH - 2.0 - SUBMENU_OVERLAP))
            .top(px(top))
            .w(px(SETTING_MENU_WIDTH))
            .p_1()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .flex()
            .flex_col()
            .children(options);
        popover_surface(flyout, cx).into_any_element()
    }

    /// MonoCode `ModelFlyout`: provider rail, search, models with stars.
    fn render_model_flyout(&self, current_key: &str, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let tab = self.composer_menus.model_tab;
        let models = self.visible_models(cx);
        let tab_button = |id: &'static str,
                          title: &'static str,
                          this_tab: ModelTab,
                          icon: AnyElement| {
            let selected = tab == this_tab;
            let hover = colors.fg.opacity(0.08);
            div()
                .id(id)
                .size(px(PROVIDER_TAB_SIZE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .when(selected, |el| el.bg(colors.fg.opacity(0.14)))
                .when(!selected, |el| el.hover(move |s| s.bg(hover)))
                .tooltip(Tooltip::text(title))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.composer_menus.model_tab != this_tab {
                        this.select_model_tab(this_tab, cx);
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.select_model_tab(this_tab, cx)))
                .child(icon)
        };
        let favorites_tab = tab_button(
            "model-tab-favorites",
            "Favorites",
            ModelTab::Favorites,
            Icon::new(IconName::Star)
                .size(IconSize::Sm)
                .color(if tab == ModelTab::Favorites {
                    colors.fg
                } else {
                    colors.fg.opacity(0.45)
                })
                .into_any_element(),
        );
        let harness_tabs = self.rail_harnesses().into_iter().map(|kind| {
            let id: &'static str = match kind {
                HarnessKind::Claude => "model-tab-claude",
                HarnessKind::Antigravity => "model-tab-antigravity",
                HarnessKind::Codex => "model-tab-codex",
                HarnessKind::Grok => "model-tab-grok",
                HarnessKind::OpenCode => "model-tab-opencode",
            };
            tab_button(
                id,
                kind.label(),
                ModelTab::Harness(kind),
                HarnessIcon::new(kind.id())
                    .size(px(16.0))
                    .into_any_element(),
            )
        });
        let rail = div()
            .w(px(44.0))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .p(px(6.0))
            .border_r_1()
            .border_color(colors.border)
            .child(favorites_tab)
            .children(harness_tabs);
        let search = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(6.0))
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::Search)
                    .size(IconSize::Xs)
                    .color(colors.fg.opacity(0.5)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .child(self.model_search_input.clone()),
            );
        let empty = if models.is_empty() {
            let query = self.model_query(cx);
            Some(
                div()
                    .px_2()
                    .py_3()
                    .text_size(px(12.0))
                    .text_color(colors.fg.opacity(0.5))
                    .child(if tab == ModelTab::Favorites && query.trim().is_empty() {
                        "No favorite models"
                    } else {
                        "No matching models"
                    }),
            )
        } else {
            None
        };
        let rows =
            models.iter().enumerate().map(|(ix, model)| {
                let key = model.key.clone();
                let highlighted = ix == self.model_picker_index;
                let favorited = self.favorite_models.contains(&key);
                let (pick_key, star_key) = (key.clone(), key.clone());
                let group = SharedString::from(format!("model-row-{key}"));
                let hover = colors.fg.opacity(0.05);
                let star_hover = colors.fg.opacity(0.08);
                let star_tip = if favorited {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
                };
                div()
                    .id(SharedString::from(format!("model-opt-{key}")))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .h(px(OPTION_HEIGHT))
                    .px_1()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_color(colors.fg)
                    .when(highlighted, |el| el.bg(colors.active))
                    .when(!highlighted, |el| el.hover(move |s| s.bg(hover)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.model_picker_index != ix {
                            this.model_picker_index = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_model(&pick_key, cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_1p5()
                            .truncate()
                            .text_size(px(13.0))
                            .child(model.label.clone()),
                    )
                    // Favorites mix providers, so each row names its own.
                    .when(tab == ModelTab::Favorites, |el| {
                        el.child(
                            div()
                                .max_w(px(96.0))
                                .flex_none()
                                .truncate()
                                .text_size(px(10.0))
                                .text_color(colors.fg.opacity(0.4))
                                .child(
                                    model
                                        .provider
                                        .clone()
                                        .unwrap_or_else(|| model.harness.label().to_string()),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("model-star-{key}")))
                            .size(px(24.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .when(!favorited, |el| {
                                el.opacity(0.0)
                                    .group_hover(group.clone(), |s| s.opacity(1.0))
                            })
                            .hover(move |s| s.bg(star_hover))
                            .tooltip(Tooltip::text(star_tip))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_favorite_model(&star_key, cx);
                            }))
                            .child(Icon::new(IconName::Star).size(IconSize::Xs).color(
                                if favorited {
                                    colors.fg.opacity(0.6)
                                } else {
                                    colors.fg.opacity(0.35)
                                },
                            )),
                    )
                    .when(key == current_key, |el| {
                        el.child(
                            div()
                                .size(px(24.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Xs)
                                        .color(colors.fg.opacity(0.55)),
                                ),
                        )
                    })
            });
        let list = div()
            .id("model-flyout-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_1()
            .children(rows)
            .children(empty);
        let flyout = div()
            .id("model-flyout")
            .absolute()
            .left(px(MENU_WIDTH - 2.0 - SUBMENU_OVERLAP))
            .bottom(px(-5.0))
            .w(px(MODEL_MENU_WIDTH))
            .h(px(model_menu_height()))
            .flex()
            .overflow_hidden()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .child(rail)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(search)
                    .child(list),
            );
        popover_surface(flyout, cx).into_any_element()
    }

    /// The recent-models menu (⌘. / right-click on the chip).
    pub(super) fn render_recent_models_popover(
        &self,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
        let current = self.current_model_key();
        let active = self.composer_menus.recent_index;
        let rows = recent_menu_models(&self.recent_models, &current)
            .into_iter()
            .enumerate()
            .map(|(ix, model)| {
                let key = model.key.clone();
                let pick_key = key.clone();
                let hover = colors.fg.opacity(0.05);
                let available = self.harness_available(model.harness);
                let subtitle = match &model.provider {
                    Some(provider) => format!("{} · {provider}", model.harness.label()),
                    None => model.harness.label().to_string(),
                };
                div()
                    .id(SharedString::from(format!("recent-model-{key}")))
                    .when(!available, |el| {
                        el.opacity(0.3)
                            .tooltip(Tooltip::text(self.harness_unavailable_hint(model.harness)))
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(40.0))
                    .px_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_color(colors.fg)
                    .when(ix == active, |el| el.bg(colors.active))
                    .when(ix != active, |el| el.hover(move |s| s.bg(hover)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.composer_menus.recent_index != ix {
                            this.composer_menus.recent_index = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_model(&pick_key, cx)))
                    .child(HarnessIcon::new(model.harness.id()).size(px(16.0)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .line_height(px(16.0))
                                    .child(model.label),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .line_height(px(16.0))
                                    .text_color(colors.fg.opacity(0.45))
                                    .child(subtitle),
                            ),
                    )
                    .when(key == current, |el| {
                        el.child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Xs)
                                .color(colors.fg.opacity(0.55)),
                        )
                    })
            });
        div()
            .id("composer-recent-models")
            .track_focus(&self.composer_menus.focus)
            .absolute()
            .bottom(px(32.0))
            .left_0()
            .w(px(MENU_WIDTH))
            .p_1()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .flex()
            .flex_col()
            .children(rows)
    }
}

/// MonoCode's menu switch: a 36×20 track with a 16px knob.
fn toggle_switch(on: bool, cx: &Context<BenCodeApp>) -> impl IntoElement {
    let colors = &cx.theme().colors;
    div()
        .relative()
        .flex_none()
        .w(px(36.0))
        .h(px(20.0))
        .rounded_full()
        .bg(colors.fg.opacity(if on { 0.35 } else { 0.15 }))
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(if on { 18.0 } else { 2.0 }))
                .size(px(16.0))
                .rounded_full()
                .bg(colors.fg)
                .shadow_sm(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_name_or_provider_within_the_tab() {
        let claude = ModelTab::Harness(HarnessKind::Claude);
        assert_eq!(filtered_models(claude, &[], "").len(), 3);
        assert_eq!(filtered_models(claude, &[], "opus").len(), 1);
        assert_eq!(filtered_models(claude, &[], "claude code").len(), 3);
        assert!(filtered_models(claude, &[], "zzzz-none").is_empty());
        let gemini = "antigravity:gemini-3.1-pro-high".to_string();
        let favorites = [gemini.clone(), "gone:model".to_string()];
        let starred = filtered_models(ModelTab::Favorites, &favorites, "");
        assert_eq!(starred.len(), 1);
        assert_eq!(starred[0].key, gemini);
    }

    #[test]
    fn recent_menu_keeps_the_current_model() {
        let recent = vec!["antigravity:gemini-3.1-pro-high".to_string()];
        let models = recent_menu_models(&recent, "claude:opus");
        let keys: Vec<_> = models.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["antigravity:gemini-3.1-pro-high", "claude:opus"]);
        assert_eq!(recent_menu_models(&[], "claude:opus").len(), 1);
    }
}
