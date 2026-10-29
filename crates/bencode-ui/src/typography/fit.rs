use gpui::{
    App, Div, IntoElement, ParentElement, Pixels, RenderOnce, ShapedLine, SharedString, Styled,
    TextAlign, Window, canvas, div, point, transparent_black,
};

pub(crate) const LEADING: f32 = 1.4;

fn shape(text: &str, window: &mut Window) -> ShapedLine {
    let style = window.text_style();
    let run = style.to_run(text.len());
    let size = style.font_size.to_pixels(window.rem_size());
    window
        .text_system()
        .shape_line(SharedString::from(text.to_string()), size, &[run], None)
}

fn width_of(text: &str, window: &mut Window) -> Pixels {
    shape(text, window).width
}

/// Longest front that fits `width` with an ellipsis after it.
fn fit_end(text: &str, width: Pixels, window: &mut Window) -> String {
    // GPUI snaps a box's edges to device pixels, so a box sized to its
    // text can end up to one device pixel narrower than the text.
    let width = width + Pixels::from(1.0 / window.scale_factor());
    if width_of(text, window) <= width {
        return text.to_string();
    }
    if width_of("…", window) > width {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |kept: usize| {
        let front: String = chars[..kept].iter().collect();
        let mut out: String = front
            .trim_end_matches(|ch: char| ch.is_whitespace() || matches!(ch, '.' | ',' | ';' | ':'))
            .into();
        out.push('…');
        out
    };
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if width_of(&cut(mid), window) <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    cut(low)
}

/// Where a line starts in a box with `free` width to spare.
fn offset(align: TextAlign, free: Pixels) -> Pixels {
    let free = free.max(Pixels::ZERO);
    match align {
        TextAlign::Left => Pixels::ZERO,
        TextAlign::Center => free / 2.,
        TextAlign::Right => free,
    }
}

/// One line, cut by `fit` to its box and aligned.
fn cut_line(text: SharedString, fit: fn(&str, Pixels, &mut Window) -> String) -> Div {
    let line = text.clone();
    div()
        .relative()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(div().text_color(transparent_black()).child(text))
        .child(
            canvas(
                move |bounds, window, _| shape(&fit(&line, bounds.size.width, window), window),
                |bounds, shaped, window, cx| {
                    let style = window.text_style();
                    let line = style.line_height_in_pixels(window.rem_size());
                    let x = offset(style.text_align, bounds.size.width - shaped.width);
                    let painted = shaped.paint(
                        bounds.origin + point(x, Pixels::ZERO),
                        line,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    if let Err(error) = painted {
                        log::error!("ellipsis: a cut line failed to paint: {error:#}");
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

/// One line in the text style around it, cut at the end with an ellipsis when its box is too narrow.
#[derive(IntoElement)]
pub struct Ellipsis {
    text: SharedString,
}

impl Ellipsis {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for Ellipsis {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        cut_line(self.text, fit_end)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TextAlign, px};

    use super::offset;

    #[test]
    fn a_line_starts_by_its_alignment() {
        assert_eq!(offset(TextAlign::Left, px(40.)), px(0.));
        assert_eq!(offset(TextAlign::Center, px(40.)), px(20.));
        assert_eq!(offset(TextAlign::Right, px(40.)), px(40.));
        assert_eq!(
            offset(TextAlign::Right, px(-8.)),
            px(0.),
            "a cut line fills its box"
        );
    }
}

#[cfg(test)]
mod fits {
    use gpui::{Pixels, TestAppContext};

    use super::{fit_end, width_of};

    #[gpui::test]
    fn an_ellipsis_keeps_the_longest_front_that_fits(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let text = "Good interfaces. Quiet ones";
            let whole = width_of(text, window);
            assert_eq!(
                fit_end(text, whole, window),
                text,
                "a line that fits stays whole"
            );
            let room = width_of("Good interfaces.…", window);
            assert_eq!(
                fit_end(text, room, window),
                "Good interfaces…",
                "no stop or space before the ellipsis"
            );
            assert_eq!(
                fit_end(text, Pixels::ZERO, window),
                "",
                "no room shows nothing"
            );
        });
    }
}
