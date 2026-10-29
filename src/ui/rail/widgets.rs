//! The navigator's small pieces: MonoCode `RailAction` / `RailSearch`, the
//! views' icons, the title-bar `IconButton`, a card's hover controls, its
//! diff stat and mascot.

use std::time::Duration;

use crate::ui::appearance::DiffColors;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, Styled, Window, div, prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::ui::mascot::{Mascot, pixel_sprite};
use crate::ui::scale::px;
use crate::ui::spinner;

/// MonoCode `h-8`: every rail row.
pub(crate) const ROW_HEIGHT: f32 = 32.0;
/// MonoCode `--mascot-beat`.
const MASCOT_BEAT: Duration = Duration::from_millis(460);
pub(super) use crate::ui::tokens::{AMBER_400, EMERALD_400};
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `formatInteger`: thousands grouped with commas.
pub(super) fn format_diff_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// MonoCode `projectCardTitle`: name, path, "Working", then the changes.
pub(super) fn project_card_title(
    name: &str,
    path: &str,
    (additions, deletions): (usize, usize),
    busy: bool,
) -> String {
    let mut parts = vec![name.to_string(), path.to_string()];
    if busy {
        parts.push("Working".into());
    }
    let changes: Vec<String> = [
        (additions > 0).then(|| format!("+{}", format_diff_number(additions))),
        (deletions > 0).then(|| format!("-{}", format_diff_number(deletions))),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !changes.is_empty() {
        parts.push(changes.join(" "));
    }
    parts.join("\n")
}

/// MonoCode `ProjectDiffStat`: `flex gap-1 text-[11px] font-semibold
/// tabular-nums`, in the chosen diff palette (`text-diff-add-fg` /
/// `text-diff-del-fg`).
pub(super) fn project_diff_stat(
    additions: usize,
    deletions: usize,
    diff: DiffColors,
) -> impl IntoElement {
    let (add, del) = (diff.add_fg, diff.del_fg);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_size(Text::Sm.px())
        .font_weight(FontWeight::SEMIBOLD)
        .when(additions > 0, |el| {
            el.child(
                div()
                    .text_color(add)
                    .child(format!("+{}", format_diff_number(additions))),
            )
        })
        .when(deletions > 0, |el| {
            el.child(
                div()
                    .text_color(del)
                    .child(format!("-{}", format_diff_number(deletions))),
            )
        })
}

/// MonoCode `ProjectMascot className="size-3"`: the rest frame, or while
/// busy (`mascot-active`) rest and talk swapped each beat with a 1px hop.
pub(super) fn project_mascot_icon(
    mascot: &'static Mascot,
    color: Hsla,
    busy: bool,
    cx: &App,
) -> AnyElement {
    mascot_icon(mascot, color, busy, 12.0, cx)
}

/// The mascot at `size` (the Agents section draws it at `size-2`).
/// Its beat is stepped by the app's clock (`ui/spinner.rs`).
pub(crate) fn mascot_icon(
    mascot: &'static Mascot,
    color: Hsla,
    busy: bool,
    size: f32,
    cx: &App,
) -> AnyElement {
    let size = px(size);
    if !busy {
        return pixel_sprite(&mascot.rest, size, color, false);
    }
    // A busy mark (`ui/marks.rs`): the sprite talks inside a box that
    // holds still.
    crate::ui::marks::mark(
        move |cx| {
            let talking = !spinner::still(cx) && spinner::cycle(MASCOT_BEAT) >= 0.5;
            div()
                .flex_none()
                .size(size)
                .child(
                    div()
                        .relative()
                        .size(size)
                        .top(px(if talking { -1.0 } else { 0.0 }))
                        .child(pixel_sprite(
                            if talking { &mascot.talk } else { &mascot.rest },
                            size,
                            color,
                            false,
                        )),
                )
                .into_any_element()
        },
        cx,
    )
}

/// A card control shown only while the card is hovered: `hidden
/// group-hover:grid place-items-center text-content/55 hover:text-content`.
pub(super) fn hover_control(
    id: SharedString,
    icon: IconName,
    size: IconSize,
    tip: &'static str,
    group: &SharedString,
    fg: Hsla,
) -> Stateful<Div> {
    div()
        .id(id.clone())
        .group(id.clone())
        .absolute()
        .flex()
        .invisible()
        .group_hover(group.clone(), |s| s.visible())
        .items_center()
        .justify_center()
        .rounded(px(if matches!(size, IconSize::Sm) {
            4.0
        } else {
            6.0
        }))
        .when(!matches!(size, IconSize::Sm), |el| {
            el.hover(move |s| s.bg(fg.fill(Fill::Selected)))
        })
        .tooltip(Tooltip::text(tip))
        .child(
            Icon::new(icon)
                .size(size)
                .color(fg.ink(Ink::Secondary))
                .group_hover_color(id, fg),
        )
}

/// A navigator row while not selected (MonoCode `ProjectCard`'s
/// `opacity-65`): the whole row, its icon and counts too.
pub(crate) const UNSELECTED_OPACITY: f32 = 0.65;

/// The selected navigator row's fill: content 14%, 8% in light (MonoCode
/// `bg-selection-strong` is 12% and 7%, too close to the hover's 5%).
pub(crate) fn selection_strong(cx: &App) -> Hsla {
    let theme = cx.theme();
    theme
        .colors
        .fg
        .opacity(if theme.is_dark() { 0.14 } else { 0.08 })
}

/// The navigator's rows, one look for its projects, Bots and threads
/// (MonoCode `ProjectCard`): `h-8 rounded-md px-2`, the selection's strong
/// fill, else the row at 65% with a `content/5` fill on hover.
pub(crate) fn nav_row(row: Stateful<Div>, selected: bool, cx: &App) -> Stateful<Div> {
    let hover = cx.theme().colors.fg.fill(Fill::Hover);
    let strong = selection_strong(cx);
    row.h(px(ROW_HEIGHT)).px_2().rounded(px(6.0)).map(|el| {
        if selected {
            el.bg(strong)
        } else {
            el.opacity(UNSELECTED_OPACITY).hover(move |s| s.bg(hover))
        }
    })
}

/// A navigator row's name: `min-w-0 truncate text-sm font-medium
/// leading-tight`, in the row's ink.
pub(crate) fn nav_label() -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(Text::Lg.px())
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.25))
}

/// The row label: a navigator row's name that takes the room left.
pub(super) fn rail_label(label: impl Into<SharedString>) -> impl IntoElement {
    nav_label().flex_1().child(label.into())
}

/// A view's icon beside the search (BenCode's own, where MonoCode lists
/// each view as a row): `size-8 rounded-md`, lit while the view is open,
/// with an accent dot for activity not read yet.
pub(super) fn rail_icon_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    active: bool,
    dot: bool,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&mut BenCodeApp, &ClickEvent, &mut Window, &mut Context<BenCodeApp>) + 'static,
) -> Stateful<Div> {
    let theme = cx.theme();
    let (fg, accent) = (theme.colors.fg, theme.colors.accent);
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    div()
        .id(id)
        .group(id)
        .relative()
        .flex()
        .flex_none()
        .size(px(ROW_HEIGHT))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .tooltip(Tooltip::text(tip))
        .map(|el| {
            if active {
                el.bg(selection)
            } else {
                el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
            }
        })
        .on_click(cx.listener(on_click))
        .child(rail_icon(id, icon, active, fg))
        .when(dot, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(accent),
            )
        })
}

/// MonoCode `RailSearch`: a `RailAction` with `border border-content/8
/// px-1.5 shadow-sm`.
pub(super) fn rail_search(active: bool, cx: &Context<BenCodeApp>) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.colors.fg;
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    let id = "rail-search";
    rail_row(id, active, selection, fg)
        .px(px(6.0))
        .border_1()
        .border_color(fg.fill(Fill::Selected))
        .shadow_sm()
        .on_click(cx.listener(|this, _, _, cx| this.toggle_search(cx)))
        .child(rail_icon(id, IconName::Search, active, fg))
        .child(rail_label("Search"))
        .child(shortcut_hint("⌘K", fg))
}

fn rail_row(id: &'static str, active: bool, selection: Hsla, fg: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .group(id)
        .relative()
        .flex()
        .w_full()
        .flex_none()
        .items_center()
        .gap_2()
        .h(px(ROW_HEIGHT))
        .rounded(px(6.0))
        .map(|el| {
            if active {
                el.bg(selection).text_color(fg)
            } else {
                el.text_color(fg.ink(Ink::Secondary))
                    .hover(move |s| s.bg(fg.fill(Fill::Strong)).text_color(fg))
            }
        })
}

/// `size-4 shrink-0 opacity-70` in the row's ink.
fn rail_icon(group: &'static str, icon: IconName, active: bool, fg: Hsla) -> impl IntoElement {
    let ink = if active { fg } else { fg.ink(Ink::Secondary) };
    Icon::new(icon)
        .size(IconSize::Md)
        .color(ink.opacity(0.7))
        .group_hover_color(group, fg.ink(Ink::Body))
}

fn shortcut_hint(keys: &'static str, fg: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(Text::Sm.px())
        .text_color(fg.ink(Ink::Muted))
        .child(keys)
}

/// MonoCode `ProjectSectionHeader`'s buttons: `grid size-5 rounded-md
/// text-content/50 hover:bg-content/8 hover:text-content`, a `size-3.5`
/// glyph, lit while open.
pub(super) fn section_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    open: bool,
    fg: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .group(id)
        .relative()
        .flex()
        .flex_none()
        .size_5()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .when(open, |el| el.bg(fg.fill(Fill::Selected)))
        .hover(move |s| s.bg(fg.fill(Fill::Selected)))
        .tooltip(Tooltip::text(tip))
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(if open { fg } else { fg.ink(Ink::Secondary) })
                .group_hover_color(id, fg),
        )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TitleButtonState {
    Normal,
    /// `aria-pressed`: `text-content`.
    Active,
    /// `text-content/25`, no hover.
    Disabled,
}

/// MonoCode `TitleBar` `IconButton`: `grid size-6.5 rounded-md`, a
/// `size-3.5` glyph at `text-content/50` that lights up over `content/10`.
pub(super) fn title_icon_button(
    id: impl Into<SharedString>,
    icon: IconName,
    tip: &'static str,
    state: TitleButtonState,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&mut BenCodeApp, &ClickEvent, &mut Window, &mut Context<BenCodeApp>) + 'static,
) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    let id: SharedString = id.into();
    let color = match state {
        TitleButtonState::Normal => fg.ink(Ink::Secondary),
        TitleButtonState::Active => fg,
        TitleButtonState::Disabled => fg.ink(Ink::Faint),
    };
    let enabled = state != TitleButtonState::Disabled;
    div()
        .id(id.clone())
        .group(id.clone())
        .flex()
        .flex_none()
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .tooltip(Tooltip::text(tip))
        .when(enabled, |el| {
            el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                .on_click(cx.listener(on_click))
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(color)
                .when(enabled, |icon| icon.group_hover_color(id, fg)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_numbers_group_thousands() {
        assert_eq!(format_diff_number(7), "7");
        assert_eq!(format_diff_number(1_234), "1,234");
        assert_eq!(format_diff_number(1_234_567), "1,234,567");
    }

    #[test]
    fn card_titles_list_path_work_and_changes() {
        assert_eq!(
            project_card_title("app", "/x/app", (1_200, 3), true),
            "app\n/x/app\nWorking\n+1,200 -3"
        );
        assert_eq!(
            project_card_title("app", "/x/app", (0, 0), false),
            "app\n/x/app"
        );
    }
}
