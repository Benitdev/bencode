//! The rail's small pieces: MonoCode `RailAction` / `RailSearch`, the
//! title-bar `IconButton`, a card's hover controls, its diff stat and
//! mascot.

use std::time::Duration;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Div, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, SharedString, Stateful, Styled, Window, div,
    prelude::*, relative,
};
use crate::ui::appearance::DiffColors;

use crate::app::BenCodeApp;
use crate::ui::mascot::{Mascot, pixel_sprite};
use crate::ui::scale::px;

/// MonoCode `h-8`: every rail row.
pub(super) const ROW_HEIGHT: f32 = 32.0;
/// MonoCode `--mascot-beat`.
const MASCOT_BEAT: Duration = Duration::from_millis(460);
/// MonoCode `text-amber-400`: the muted badge.
pub(super) const AMBER_400: u32 = 0xfbbf24;

/// MonoCode `formatInteger`: thousands grouped with commas.
pub(super) fn format_diff_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// MonoCode `projectCardTitle`: name, path, "Working", then the changes.
pub(super) fn project_card_title(name: &str, path: &str, (additions, deletions): (usize, usize), busy: bool) -> String {
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
pub(super) fn project_diff_stat(additions: usize, deletions: usize, diff: DiffColors) -> impl IntoElement {
    let (add, del) = (diff.add_fg, diff.del_fg);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .when(additions > 0, |el| {
            el.child(div().text_color(add).child(format!("+{}", format_diff_number(additions))))
        })
        .when(deletions > 0, |el| {
            el.child(div().text_color(del).child(format!("-{}", format_diff_number(deletions))))
        })
}

/// MonoCode `ProjectMascot className="size-3"`: the rest frame, or while
/// busy (`mascot-active`) rest and talk swapped each beat with a 1px hop.
pub(super) fn project_mascot_icon(mascot: &'static Mascot, color: Hsla, busy: bool, id: &str) -> AnyElement {
    mascot_icon(mascot, color, busy, id, 12.0)
}

/// The mascot at `size` (the Working agents card draws it at `size-2`).
pub(super) fn mascot_icon(mascot: &'static Mascot, color: Hsla, busy: bool, id: &str, size: f32) -> AnyElement {
    let size = px(size);
    if !busy {
        return pixel_sprite(&mascot.rest, size, color, false);
    }
    div()
        .relative()
        .size(size)
        .with_animation(
            SharedString::from(format!("{id}-mascot")),
            Animation::new(MASCOT_BEAT).repeat(),
            move |el, delta| {
                let talking = delta >= 0.5;
                el.top(px(if talking { -1.0 } else { 0.0 })).child(pixel_sprite(
                    if talking { &mascot.talk } else { &mascot.rest },
                    size,
                    color,
                    false,
                ))
            },
        )
        .into_any_element()
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
        .rounded(px(if matches!(size, IconSize::Sm) { 4.0 } else { 6.0 }))
        .when(!matches!(size, IconSize::Sm), |el| el.hover(move |s| s.bg(fg.opacity(0.08))))
        .tooltip(Tooltip::text(tip))
        .child(Icon::new(icon).size(size).color(fg.opacity(0.55)).group_hover_color(id, fg))
}

/// What a rail row shows at its end.
pub(super) enum RailTrailing {
    None,
    /// `size-2 rounded-full bg-accent`: activity not read yet.
    Dot,
    /// `text-[11px] text-content/40`.
    Shortcut(&'static str),
}

/// The row label: `min-w-0 flex-1 truncate text-sm font-medium leading-tight`.
pub(super) fn rail_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .min_w_0()
        .flex_1()
        .truncate()
        .text_size(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.25))
        .child(label.into())
}

/// MonoCode `RailAction`: `flex h-8 items-center gap-2 rounded-md px-2`,
/// `bg-selection text-content` when active, else `text-content/50
/// hover:bg-content/10 hover:text-content`, its `size-4` icon at 70%.
pub(super) fn rail_action(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    active: bool,
    trailing: RailTrailing,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&mut BenCodeApp, &ClickEvent, &mut Window, &mut Context<BenCodeApp>) + 'static,
) -> Stateful<Div> {
    let theme = cx.theme();
    let (fg, accent) = (theme.colors.fg, theme.colors.accent);
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    rail_row(id, active, selection, fg)
        .px_2()
        .on_click(cx.listener(on_click))
        .child(rail_icon(id, icon, active, fg))
        .child(rail_label(label))
        .map(|el| match trailing {
            RailTrailing::None => el,
            RailTrailing::Dot => el.child(div().flex_none().size_2().rounded_full().bg(accent)),
            RailTrailing::Shortcut(keys) => el.child(shortcut_hint(keys, fg)),
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
        .border_color(fg.opacity(0.08))
        .shadow_sm()
        .on_click(cx.listener(|this, _, _, cx| this.open_search_modal(cx)))
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
                el.text_color(fg.opacity(0.5))
                    .hover(move |s| s.bg(fg.opacity(0.10)).text_color(fg))
            }
        })
}

/// `size-4 shrink-0 opacity-70` in the row's ink.
fn rail_icon(group: &'static str, icon: IconName, active: bool, fg: Hsla) -> impl IntoElement {
    let ink = if active { fg } else { fg.opacity(0.5) };
    Icon::new(icon)
        .size(IconSize::Md)
        .color(ink.opacity(0.7))
        .group_hover_color(group, fg.opacity(0.7))
}

fn shortcut_hint(keys: &'static str, fg: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(11.0))
        .text_color(fg.opacity(0.4))
        .child(keys)
}

/// MonoCode `ProjectSectionHeader`'s buttons: `grid size-5 rounded-md
/// text-content/50 hover:bg-content/8 hover:text-content`, a `size-3.5`
/// glyph, lit while open.
pub(super) fn section_button(id: &'static str, icon: IconName, tip: &'static str, open: bool, fg: Hsla) -> Stateful<Div> {
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
        .when(open, |el| el.bg(fg.opacity(0.08)))
        .hover(move |s| s.bg(fg.opacity(0.08)))
        .tooltip(Tooltip::text(tip))
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(if open { fg } else { fg.opacity(0.5) })
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
        TitleButtonState::Normal => fg.opacity(0.5),
        TitleButtonState::Active => fg,
        TitleButtonState::Disabled => fg.opacity(0.25),
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
            el.hover(move |s| s.bg(fg.opacity(0.10)))
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
        assert_eq!(project_card_title("app", "/x/app", (0, 0), false), "app\n/x/app");
    }
}
