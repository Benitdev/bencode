//! The composer's inline effort scale uses the options supplied by the model.
use bencode_ui::{
    forms::Slider,
    theme::{ActiveTheme, Mix},
};
use gpui::{Context, ParentElement, SharedString, Styled, div, prelude::*};
use serde_json::{Map, Value};

use crate::{
    app::BenCodeApp,
    harness::catalog::ModelSetting,
    ui::{
        scale::px,
        tokens::{Fill, Text, Tone as _},
    },
};

pub(super) const ROW_HEIGHT: f32 = 56.0;

/// Keep the choice list for an unknown saved value or a model without a range.
pub(super) fn index(setting: &ModelSetting, values: Option<&Map<String, Value>>) -> Option<usize> {
    if !setting.is_effort() || setting.options.len() < 2 {
        return None;
    }
    let value = setting.compatible(setting.value(values))?;
    setting.options.iter().position(|(v, _)| v == value)
}

impl BenCodeApp {
    pub(super) fn effort_control(
        &self,
        setting: &ModelSetting,
        index: usize,
        active: bool,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let colors = &cx.theme().colors;
        let accent = colors.accent;
        let weak = cx.entity().downgrade();
        let setting = setting.clone();
        let label = setting.options[index].1.clone();
        let name = setting.menu_label().to_owned();
        let last_index = (setting.options.len() - 1) as f64;
        let ratio = index as f32 / last_index as f32;
        let slider = Slider::new(
            SharedString::from(format!("effort-slider-{}", setting.id)),
            index as f64,
        )
        .range(0.0, last_index)
        .spectrum(name.clone(), label.clone())
        .on_change(move |index, _, cx| {
            if let Some((value, _)) = setting.options.get(index.round() as usize)
                && let Err(err) = weak.update(cx, |this, cx| {
                    this.set_model_setting(&setting.id, value, cx)
                })
            {
                log::debug!("effort changed after app drop: {err:#}");
            }
        });
        div()
            .h(px(ROW_HEIGHT))
            .px_2()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(8.0))
            .when(active, |el| el.bg(colors.fg.fill(Fill::Selected)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_size(Text::Md.px())
                    .line_height(px(16.0))
                    .text_color(colors.fg)
                    .child(name)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(accent.mix(&colors.fg, 0.22 + ratio * 0.22))
                            .child(label),
                    ),
            )
            .child(slider)
    }
}

#[cfg(test)]
mod tests {
    use super::index;
    use crate::harness::catalog::ModelSetting;
    use serde_json::{Map, Value};

    #[test]
    fn effort_index_resolves_options_and_aliases() {
        let effort = ModelSetting::select(
            "effort",
            "Reasoning",
            "high",
            &[
                ("low", "Low"),
                ("medium", "Medium"),
                ("high", "High"),
                ("xhigh", "Extra High"),
            ],
        );
        assert_eq!(index(&effort, None), Some(2));

        let mut values = Map::new();
        values.insert("effort".into(), Value::String("extra-high".into()));
        assert_eq!(index(&effort, Some(&values)), Some(3));

        values.insert("effort".into(), Value::String("unknown".into()));
        assert_eq!(index(&effort, Some(&values)), None);

        let toggle = ModelSetting::toggle("thinking", "Thinking");
        assert_eq!(index(&toggle, None), None);

        let single = ModelSetting::select("effort", "Reasoning", "high", &[("high", "High")]);
        assert_eq!(index(&single, None), None);
    }
}
