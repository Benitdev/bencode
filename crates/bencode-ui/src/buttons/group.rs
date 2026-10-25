use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div};

use super::{Button, button::Slot};

/// Buttons joined edge to edge.
#[derive(IntoElement, Default)]
pub struct ButtonGroup {
    buttons: Vec<Button>,
}

impl ButtonGroup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }
}

impl RenderOnce for ButtonGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let count = self.buttons.len();
        assert!(count >= 2, "a button group joins two buttons or more");
        div().flex().children(
            self.buttons
                .into_iter()
                .enumerate()
                .map(|(ix, mut button)| {
                    button.slot = Some(match ix {
                        0 => Slot::First,
                        ix if ix + 1 == count => Slot::Last,
                        _ => Slot::Middle,
                    });
                    button
                }),
        )
    }
}
