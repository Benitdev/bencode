use std::ops::Range;

use gpui::{
    App, FontWeight, HighlightStyle, IntoElement, Keystroke, ParentElement, RenderOnce,
    SharedString, Styled, StyledText, Window, div,
};
use regex::RegexBuilder;

use super::keys::{keystroke, keystroke_labels};
use crate::theme::{ActiveTheme, Platform, Radius, TextSize};

fn cap(label: String, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    div()
        .min_w(theme.text_size(TextSize::Xs) * 1.8)
        .px_1()
        .flex()
        .justify_center()
        .rounded(theme.radius(Radius::Sm))
        .border_1()
        .border_b_2()
        .border_color(theme.colors.border_strong)
        .bg(theme.colors.surface)
        .text_size(theme.text_size(TextSize::Xs))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.colors.fg_muted)
        .child(label)
}

/// One key, labeled for the theme's platform. Takes gpui key syntax.
#[derive(IntoElement)]
pub struct Kbd {
    source: SharedString,
}

impl Kbd {
    /// `"secondary-s"` reads ⌘S for the Mac and Ctrl S for the others.
    pub fn new(source: &str) -> Self {
        Self {
            source: SharedString::from(source.to_string()),
        }
    }
}

impl RenderOnce for Kbd {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let platform = cx.theme().platform;
        let labels = keystroke_labels(&keystroke(&self.source, platform), platform);
        div()
            .flex()
            .gap_0p5()
            .children(labels.into_iter().map(|label| cap(label, cx)))
    }
}

/// A chord sequence such as `"cmd-k cmd-s"`.
#[derive(IntoElement)]
pub struct KbdCombo {
    source: SharedString,
}

impl KbdCombo {
    pub fn new(source: &str) -> Self {
        Self {
            source: SharedString::from(source.to_string()),
        }
    }
}

impl RenderOnce for KbdCombo {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let platform = cx.theme().platform;
        let strokes: Vec<Keystroke> = self
            .source
            .split_whitespace()
            .map(|source| keystroke(source, platform))
            .collect();
        let subtle = cx.theme().colors.fg_subtle;
        let spelled = platform != Platform::Mac;
        div()
            .flex()
            .items_center()
            .gap_2()
            .children(strokes.iter().map(|stroke| {
                let labels = keystroke_labels(stroke, platform);
                let last = labels.len() - 1;
                div().flex().items_center().gap_0p5().children(
                    labels.into_iter().enumerate().flat_map(|(ix, label)| {
                        let mut parts = vec![cap(label, cx).into_any_element()];
                        if spelled && ix < last {
                            parts.push(div().text_color(subtle).child("+").into_any_element());
                        }
                        parts
                    }),
                )
            }))
    }
}

/// Text with marked ranges, as for search matches.
#[derive(IntoElement)]
pub struct Highlight {
    text: SharedString,
    ranges: Vec<Range<usize>>,
}

impl Highlight {
    /// Marks every case-insensitive occurrence of `query`.
    pub fn matching(text: impl Into<SharedString>, query: &str) -> Self {
        let text = text.into();
        let ranges = if query.is_empty() {
            Vec::new()
        } else {
            match RegexBuilder::new(&regex::escape(query))
                .case_insensitive(true)
                .build()
            {
                Ok(found) => found.find_iter(&text).map(|found| found.range()).collect(),
                Err(error) => {
                    log::error!(
                        "highlight: no marks for a {}-byte query: {error}",
                        query.len()
                    );
                    Vec::new()
                }
            }
        };
        Self { text, ranges }
    }
}

impl RenderOnce for Highlight {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let mark = HighlightStyle {
            background_color: Some(colors.warning_subtle),
            color: Some(colors.fg),
            ..Default::default()
        };
        StyledText::new(self.text)
            .with_highlights(self.ranges.into_iter().map(|range| (range, mark)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Highlight, Range};

    #[test]
    fn a_query_past_the_regex_limit_marks_nothing() {
        let long = "a".repeat(500_000);
        assert!(Highlight::matching("aaa", &long).ranges.is_empty());
    }

    #[test]
    fn matching_finds_every_case_insensitive_hit() {
        let found = Highlight::matching("Ben, ben, BEN and bénit", "ben");
        assert_eq!(found.ranges, [0..3, 5..8, 10..13]);
        let none = Highlight::matching("abc", "");
        assert!(none.ranges.is_empty());
        let literal = Highlight::matching("a.b ab", ".");
        assert_eq!(literal.ranges, vec![Range { start: 1, end: 2 }]);
    }
}
