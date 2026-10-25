use std::rc::Rc;

use gpui::{App, ElementId, IntoElement, RenderOnce, Window};

use crate::{buttons::SegmentedControl, i18n, primitives::IconName};

/// How an app picks light or dark: always one, or as the system does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
    System,
}

impl Appearance {
    /// Each with its value, catalog key and icon.
    const ALL: [(Appearance, &'static str, IconName); 3] = [
        (Appearance::Light, "theme.light", IconName::Sun),
        (Appearance::Dark, "theme.dark", IconName::Moon),
        (Appearance::System, "theme.system", IconName::Monitor),
    ];
}

type OnAppearance = Rc<dyn Fn(Appearance, &mut Window, &mut App)>;

/// Light, dark, or as the system does, as three segments with their icons. The owner applies the choice, as through `Theme::set_mode`.
#[derive(IntoElement)]
pub struct ThemeSelector {
    id: ElementId,
    appearance: Appearance,
    on_change: OnAppearance,
}

impl ThemeSelector {
    pub fn new(
        id: impl Into<ElementId>,
        appearance: Appearance,
        on_change: impl Fn(Appearance, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            appearance,
            on_change: Rc::new(on_change),
        }
    }
}

impl RenderOnce for ThemeSelector {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_change = self.on_change;
        let (_, words, _) = Appearance::ALL
            .iter()
            .find(|(each, _, _)| *each == self.appearance)
            .expect("every appearance is offered");
        Appearance::ALL
            .iter()
            .fold(
                SegmentedControl::new(self.id, *words),
                |control, (_, words, icon)| {
                    control.segment(*words, i18n::text(cx, words, &[]), Some(*icon))
                },
            )
            .on_change(move |value, window, cx| {
                let (chosen, _, _) = Appearance::ALL
                    .iter()
                    .find(|(_, words, _)| *words == value.as_ref())
                    .expect("an appearance the control offers");
                log::info!("theme selector: {chosen:?}");
                on_change(*chosen, window, cx);
            })
    }
}
