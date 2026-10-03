//! The composer's model picker (MonoCode `ModelPicker`): a search field on
//! top, models grouped by harness, ↑/↓ to move, Enter to pick, Esc to
//! close, ⌘. to toggle.

use ely_gpui_component::forms::Input;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};

use super::HARNESS_ORDER;
use crate::app::BenCodeApp;
use crate::harness::catalog::{self, ModelOption};
use crate::ui::HarnessIcon;

/// Models matching `query` (label, key or harness name), in picker order.
pub fn filtered_models(query: &str) -> Vec<&'static ModelOption> {
    let query = query.trim().to_lowercase();
    HARNESS_ORDER
        .iter()
        .flat_map(|&kind| catalog::models_for(kind))
        .filter(|m| {
            query.is_empty()
                || m.label.to_lowercase().contains(&query)
                || m.key.to_lowercase().contains(&query)
                || m.harness.label().to_lowercase().contains(&query)
        })
        .collect()
}

impl BenCodeApp {
    fn model_query(&self, cx: &Context<Self>) -> String {
        self.model_search_input.read(cx).text().to_string()
    }

    /// Opens the picker on the current model with an empty search, or closes it.
    pub fn toggle_model_picker(&mut self, cx: &mut Context<Self>) {
        if self.is_model_picker_open {
            self.close_model_picker(cx);
            return;
        }
        self.is_plus_menu_open = false;
        self.is_permission_picker_open = false;
        self.is_branch_picker_open = false;
        self.is_model_picker_open = true;
        self.model_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        let current = self
            .selected_session()
            .map_or(self.selected_model.clone(), |s| s.model.clone());
        self.model_picker_index = filtered_models("")
            .iter()
            .position(|m| m.key == current)
            .unwrap_or(0);
        let focus = self.model_search_input.read(cx).focus_handle(cx);
        cx.defer(move |cx| {
            let Some(window) = cx.active_window() else {
                return;
            };
            if let Err(err) = window.update(cx, |_, window, cx| window.focus(&focus, cx)) {
                log::warn!("model picker: could not focus search: {err:#}");
            }
        });
        cx.notify();
    }

    pub fn close_model_picker(&mut self, cx: &mut Context<Self>) {
        self.is_model_picker_open = false;
        let focus = self.prompt_input.read(cx).focus_handle(cx);
        cx.defer(move |cx| {
            if let Some(window) = cx.active_window()
                && let Err(err) = window.update(cx, |_, window, cx| window.focus(&focus, cx))
            {
                log::debug!("model picker: could not refocus composer: {err:#}");
            }
        });
        cx.notify();
    }

    /// ↑/↓ in the search field, wrapping.
    pub fn move_model_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = filtered_models(&self.model_query(cx)).len() as isize;
        if len > 0 {
            self.model_picker_index =
                (self.model_picker_index as isize + delta).rem_euclid(len) as usize;
            cx.notify();
        }
    }

    /// Enter: picks the highlighted model.
    pub fn pick_highlighted_model(&mut self, cx: &mut Context<Self>) {
        let models = filtered_models(&self.model_query(cx));
        if let Some(model) = models.get(self.model_picker_index).or(models.first()) {
            self.set_session_model(model.key, cx);
        }
        self.close_model_picker(cx);
    }

    pub(super) fn render_model_picker_popover(
        &self,
        current_key: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let models = filtered_models(&self.model_query(cx));
        let rows = models.iter().enumerate().map(|(ix, model)| {
            let key = model.key;
            let highlighted = ix == self.model_picker_index;
            let hover = colors.fg.opacity(0.08);
            div()
                .id(SharedString::from(format!("model-opt-{key}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(30.0))
                .px_2()
                .rounded(px(6.0))
                .cursor_pointer()
                .when(highlighted, |el| el.bg(colors.active))
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_session_model(key, cx);
                    this.close_model_picker(cx);
                }))
                .child(HarnessIcon::new(model.harness.id()).size(px(14.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.0))
                        .child(model.label),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(colors.fg_muted)
                        .child(model.harness.label()),
                )
                .when(key == current_key, |el| {
                    el.child(
                        Icon::new(IconName::Check)
                            .size(IconSize::Xs)
                            .color(colors.accent),
                    )
                })
        });
        div()
            .id("composer-model-popover")
            .absolute()
            .bottom(px(40.0))
            .left(px(34.0))
            .w(px(310.0))
            .p_1()
            .rounded(px(8.0))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_1()
                    .pt_1()
                    .child(Input::new(&self.model_search_input)),
            )
            .child(
                div()
                    .id("composer-model-list")
                    .max_h(px(280.0))
                    .overflow_y_scroll()
                    .children(rows)
                    .when(models.is_empty(), |el| {
                        el.child(
                            div()
                                .px_2()
                                .py_2()
                                .text_size(px(12.0))
                                .text_color(colors.fg_muted)
                                .child("No matching models"),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_label_key_or_harness() {
        assert!(!filtered_models("").is_empty());
        assert!(
            filtered_models("opus")
                .iter()
                .all(|m| { m.label.to_lowercase().contains("opus") || m.key.contains("opus") })
        );
        assert!(filtered_models("zzzz-none").is_empty());
    }
}
