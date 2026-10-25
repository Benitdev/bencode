use gpui::{
    App, Bounds, Corners, ElementId, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Rems, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Window, canvas, div, fill, point, prelude::*, size,
};

use super::super::options::Run;
use crate::{
    primitives::{FocusRing, Tooltip},
    theme::{ActiveTheme, ControlSize, Radius},
};

/// Checker cells over `bounds`, at least `round` wide; odd counts keep the corners light.
pub(crate) fn dark_cells(bounds: Bounds<Pixels>, round: Pixels) -> Vec<Bounds<Pixels>> {
    let (width, height) = (bounds.size.width, bounds.size.height);
    if width <= Pixels::ZERO || height <= Pixels::ZERO {
        return Vec::new();
    }
    let least = (width.min(height) / 4.0).max(round);
    let odd = |extent: Pixels| {
        let count = ((extent / least).floor() as usize).max(1);
        if count.is_multiple_of(2) {
            count - 1
        } else {
            count
        }
    };
    let (columns, rows) = (odd(width), odd(height));
    let cell = size(width / columns as f32, height / rows as f32);
    (0..rows)
        .flat_map(|row| {
            (1 - row % 2..columns).step_by(2).map(move |column| {
                let origin = point(
                    bounds.left() + cell.width * column as f32,
                    bounds.top() + cell.height * row as f32,
                );
                Bounds::new(origin, cell)
            })
        })
        .collect()
}

/// A checkerboard filling its parent with `radius` corners, so see-through colors read as see-through.
pub(crate) fn checker(radius: Rems, cx: &App) -> impl IntoElement + use<> {
    let (light, dark) = (cx.theme().colors.surface, cx.theme().colors.active);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let round = radius
                .to_pixels(window.rem_size())
                .min(bounds.size.width.min(bounds.size.height) / 2.0);
            window.paint_quad(fill(bounds, light).corner_radii(Corners::all(round)));
            for cell in dark_cells(bounds, round) {
                window.paint_quad(fill(cell, dark));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A patch of one color. Chosen swatches wear an ink ring with a gap.
#[derive(IntoElement)]
pub struct ColorSwatch {
    id: ElementId,
    color: Hsla,
    size: ControlSize,
    selected: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Run>,
}

impl ColorSwatch {
    pub fn new(id: impl Into<ElementId>, color: impl Into<Hsla>) -> Self {
        Self {
            id: id.into(),
            color: color.into(),
            size: ControlSize::Sm,
            selected: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for ColorSwatch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (see_through, radius) = (self.color.a < 1.0, theme.radius(Radius::Sm));
        let patch = div()
            .relative()
            .size_full()
            .when(see_through, |patch| patch.child(checker(radius, cx)))
            .child(div().absolute().size_full().rounded(radius).bg(self.color));
        div()
            .id(self.id)
            .flex_none()
            .size(theme.control_height(self.size))
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(if self.selected {
                colors.accent
            } else {
                colors.border
            })
            .when(self.selected, |swatch| swatch.p_0p5())
            .when_some(self.on_click, |swatch, click| {
                swatch
                    .tab_index(0)
                    .focus_ring(cx)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |_, window, cx| click(window, cx))
            })
            .when_some(self.tooltip, |swatch, text| {
                swatch.tooltip(Tooltip::text(text))
            })
            .child(patch)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, point, px, size};

    use super::dark_cells;

    #[test]
    fn empty_boxes_get_no_cells() {
        let flat = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(40.0)));
        assert!(dark_cells(flat, px(4.0)).is_empty());
    }

    #[test]
    fn dark_cells_stay_clear_of_rounded_corners() {
        for (width, height) in [(22.0, 22.0), (244.0, 12.0), (344.0, 28.0), (31.0, 9.0)] {
            let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(width), px(height)));
            let round = px(4.0).min(bounds.size.height / 2.0);
            let cells = dark_cells(bounds, round);
            assert!(!cells.is_empty(), "{width}x{height} has a pattern");
            let corners = [
                bounds.origin,
                point(bounds.right() - round, bounds.top()),
                point(bounds.left(), bounds.bottom() - round),
                point(bounds.right() - round, bounds.bottom() - round),
            ]
            .map(|origin| Bounds::new(origin, size(round, round)));
            for cell in &cells {
                assert!(
                    bounds.contains(&cell.center()),
                    "{cell:?} inside {bounds:?}"
                );
                for corner in &corners {
                    let overlap = cell.intersect(corner);
                    assert!(
                        overlap.size.width <= px(0.001) || overlap.size.height <= px(0.001),
                        "{cell:?} reaches corner {corner:?} of {width}x{height}"
                    );
                }
            }
        }
    }
}
