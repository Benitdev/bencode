//! MonoCode `SettingsView`'s `PageHeader`, `Group` and `Row`: a page is a
//! title with a line on what it holds, then a handful of titled cards, so
//! it reads as a few topics instead of one long list of switches.
//! BenCode centres a row's control on it, where MonoCode tops it.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, App, Div, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, prelude::*, relative,
};

use crate::ui::scale::px;

/// `text-content/45`: descriptions and asides.
const QUIET: f32 = 0.45;
/// `leading-relaxed`.
const RELAXED: f32 = 1.625;

/// MonoCode `PageHeader` over the page's groups.
#[derive(IntoElement)]
pub struct SettingsPage {
    title: SharedString,
    description: Option<SharedString>,
    groups: Vec<AnyElement>,
}

impl SettingsPage {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            groups: Vec::new(),
        }
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn group(mut self, group: impl IntoElement) -> Self {
        self.groups.push(group.into_any_element());
        self
    }
}

impl RenderOnce for SettingsPage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        // `pb-4`, `text-[20px] font-semibold leading-tight`; `mt-1.5 max-w-xl
        // text-[13px] leading-relaxed text-content/45`. Groups: `pt-8 first:pt-0`.
        div()
            .flex()
            .flex_col()
            .text_color(fg)
            .child(
                div()
                    .pb_4()
                    .child(
                        div()
                            .text_size(px(20.0))
                            .line_height(relative(1.25))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .children(self.description.map(|text| {
                        div()
                            .mt(px(6.0))
                            .max_w(px(576.0))
                            .text_size(px(13.0))
                            .line_height(relative(RELAXED))
                            .text_color(fg.opacity(QUIET))
                            .child(text)
                    })),
            )
            .child(div().flex().flex_col().gap_8().children(self.groups))
    }
}

/// MonoCode `Group`: a title, a line of context and an optional action
/// above a bordered card of rows.
#[derive(IntoElement)]
pub struct SettingsGroup {
    title: SharedString,
    description: Option<SharedString>,
    action: Option<AnyElement>,
    rows: Vec<AnyElement>,
}

impl SettingsGroup {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            action: None,
            rows: Vec::new(),
        }
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Drawn at the end of the title line (Refresh, Add, a picker).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn row(mut self, row: impl IntoElement) -> Self {
        self.rows.push(row.into_any_element());
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.rows.extend(rows.into_iter().map(IntoElement::into_any_element));
        self
    }

    /// A line of quiet text in the card: an empty, loading or note state.
    pub fn note(self, text: impl Into<SharedString>) -> Self {
        self.row(SettingsNote(text.into()))
    }
}

impl RenderOnce for SettingsGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        // `flex items-end gap-4 pb-2.5`; `text-[13px] font-semibold`; `mt-1
        // text-[12px] leading-relaxed text-content/45`; action `pb-0.5`.
        let head = div()
            .flex()
            .items_end()
            .gap_4()
            .pb(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .children(self.description.map(|text| {
                        div()
                            .mt_1()
                            .text_size(px(12.0))
                            .line_height(relative(RELAXED))
                            .text_color(fg.opacity(QUIET))
                            .child(text)
                    })),
            )
            .children(self.action.map(|action| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pb(px(2.0))
                    .child(action)
            }));
        // `overflow-hidden rounded-xl border border-content/10 bg-content/3`;
        // rows `border-b border-content/5 last:border-b-0`.
        let rows = self.rows.into_iter().enumerate().map(|(ix, row)| {
            div()
                .when(ix > 0, |el| el.border_t_1().border_color(fg.opacity(0.05)))
                .child(row)
        });
        div().flex().flex_col().child(head).child(
            div()
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(12.0))
                .border_1()
                .border_color(fg.opacity(0.1))
                .bg(fg.opacity(0.03))
                .children(rows),
        )
    }
}

/// MonoCode `Row`: a label and what it does, the control at the end.
#[derive(IntoElement)]
pub struct SettingsRow {
    label: SharedString,
    aside: Option<SharedString>,
    leading: Option<AnyElement>,
    description: Option<SharedString>,
    error: bool,
    control: Option<AnyElement>,
}

impl SettingsRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            aside: None,
            leading: None,
            description: None,
            error: false,
            control: None,
        }
    }

    /// Monospace text after the label (a version, a key).
    pub fn aside(mut self, text: impl Into<SharedString>) -> Self {
        self.aside = Some(text.into());
        self
    }

    /// Drawn before the label, usually an [`icon_tile`].
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// The description, in the danger colour.
    pub fn error(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self.error = true;
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let description_color = if self.error { colors.danger } else { fg.opacity(QUIET) };
        let mono = cx.theme().mono_family.clone();
        // `flex gap-6 px-4 py-3.5`; label `text-[13px] font-medium`; control
        // `flex min-w-0 max-w-[60%] shrink-0 flex-wrap items-center
        // justify-end gap-2`.
        div()
            .flex()
            .items_center()
            .gap_6()
            .px_4()
            .py(px(14.0))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .children(self.leading)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(self.label),
                                    )
                                    .children(self.aside.map(|aside| {
                                        div()
                                            .flex_none()
                                            .font_family(mono)
                                            .text_size(px(12.0))
                                            .text_color(fg.opacity(QUIET))
                                            .child(aside)
                                    })),
                            )
                            .children(self.description.map(|text| {
                                div()
                                    .mt_1()
                                    .text_size(px(12.0))
                                    .line_height(relative(RELAXED))
                                    .text_color(description_color)
                                    .child(text)
                            })),
                    ),
            )
            .children(self.control.map(|control| {
                div()
                    .flex()
                    .flex_none()
                    .min_w_0()
                    .max_w(relative(0.6))
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(control)
            }))
    }
}

/// MonoCode's `px-4 py-3.5 text-[12px] text-content/45` paragraph.
#[derive(IntoElement)]
struct SettingsNote(SharedString);

impl RenderOnce for SettingsNote {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .px_4()
            .py(px(14.0))
            .text_size(px(12.0))
            .line_height(relative(RELAXED))
            .text_color(cx.theme().colors.fg.opacity(QUIET))
            .child(self.0)
    }
}

/// A 28px rounded tile for a row's icon (`size-7 rounded-lg bg-content/5
/// border border-content/6`).
pub fn icon_tile(icon: impl IntoElement, fg: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(8.0))
        .bg(fg.opacity(0.05))
        .border_1()
        .border_color(fg.opacity(0.06))
        .child(icon)
}
