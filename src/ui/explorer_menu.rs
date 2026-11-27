//! MonoCode `ExplorerMenu`: the context menu used across the app, drawn at
//! the pointer. Rows can carry a description, a shortcut, and a danger tint;
//! ↑/↓ move over the rows (wrapping), Enter or Space picks, Esc closes, and
//! a mouse-down anywhere else dismisses it.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnimationExt, AnyElement, App, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, SharedString, Styled, anchored, deferred, div, prelude::*, relative, rgb,
};

use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

#[derive(Clone, Debug, PartialEq)]
pub struct MenuAction {
    pub id: &'static str,
    pub label: String,
    pub description: Option<String>,
    pub shortcut: Option<SharedString>,
    pub disabled: bool,
    pub danger: bool,
    /// MonoCode `checked`: a check at the row's end.
    pub checked: bool,
    /// What the row acts on when its `id` is shared (e.g. a folder id).
    pub value: Option<String>,
    /// A leading `size-3.5` icon, spinning while `spin` is set.
    pub icon: Option<IconName>,
    pub spin: bool,
    /// MonoCode `title`: shown on hover, so the row keeps its `h-7`.
    pub tooltip: Option<String>,
    /// Opens a submenu beside it: a `ChevronRight size-3.5` at its end.
    pub submenu: bool,
}

impl MenuAction {
    pub fn new(id: &'static str, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            description: None,
            shortcut: None,
            disabled: false,
            danger: false,
            checked: false,
            value: None,
            icon: None,
            spin: false,
            tooltip: None,
            submenu: false,
        }
    }

    pub fn submenu(mut self) -> Self {
        self.submenu = true;
        self
    }

    pub fn icon(mut self, icon: IconName, spin: bool) -> Self {
        self.icon = Some(icon);
        self.spin = spin;
        self
    }

    pub fn tooltip(mut self, text: Option<String>) -> Self {
        self.tooltip = text;
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn description(mut self, text: Option<String>) -> Self {
        self.description = text;
        self
    }

    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    Item(MenuAction),
    Separator,
}

/// The rows a key can land on, in order.
fn item_indices(entries: &[MenuEntry]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, MenuEntry::Item(_)))
        .map(|(ix, _)| ix)
        .collect()
}

/// MonoCode `itemIndexAt(items, 0, 1)`: the first row.
pub fn first_item(entries: &[MenuEntry]) -> usize {
    item_indices(entries).first().copied().unwrap_or(0)
}

/// MonoCode `move`: the next row in `dir`, wrapping, disabled rows included.
pub fn step(entries: &[MenuEntry], active: usize, dir: isize) -> usize {
    let rows = item_indices(entries);
    if rows.is_empty() {
        return active;
    }
    let from = rows.iter().position(|ix| *ix == active).unwrap_or(0) as isize;
    rows[(from + dir).rem_euclid(rows.len() as isize) as usize]
}

/// The picked row's id, unless it is disabled.
pub fn pick(entries: &[MenuEntry], active: usize) -> Option<&'static str> {
    match entries.get(active) {
        Some(MenuEntry::Item(item)) if !item.disabled => Some(item.id),
        _ => None,
    }
}

/// The picked row itself, unless it is disabled.
pub fn pick_action(entries: &[MenuEntry], active: usize) -> Option<&MenuAction> {
    match entries.get(active) {
        Some(MenuEntry::Item(item)) if !item.disabled => Some(item),
        _ => None,
    }
}

/// Where the menu opens.
#[derive(Clone, Copy, Debug)]
pub enum MenuPlace {
    /// At a window point, as a context menu does.
    At(Point<Pixels>),
    /// Above the element it is placed in (MonoCode `Popover side="top"`).
    Above,
    /// Under the element it is placed in (`Popover side="bottom"`).
    Below,
    /// Top-right corner at a window point, growing left with its rows;
    /// `width` is then a floor (MonoCode `absolute top-full right-0 min-w-*`).
    UnderRight(Point<Pixels>),
}

pub struct MenuView<'a> {
    pub id: &'static str,
    pub entries: &'a [MenuEntry],
    pub active: usize,
    pub place: MenuPlace,
    pub width: f32,
    pub focus: &'a gpui::FocusHandle,
    /// Drawn above the rows (MonoCode's folder colour swatches).
    pub header: Option<AnyElement>,
}

/// MonoCode `text-red-300` / `bg-red-500`: the danger row's ink and fills.
const RED_300: u32 = 0xfca5a5;
const RED_500: u32 = 0xef4444;

/// MonoCode `ExplorerMenu` rows: `gap-3 rounded-lg px-2 text-[13px]
/// leading-none`, `h-7`.
const ROW_RADIUS: f32 = 8.0;
const ROW_PAD_X: f32 = 8.0;
const ROW_GAP: f32 = 12.0;
const ROW_TEXT: f32 = 13.0;
const ROW_HEIGHT: f32 = 28.0;

/// Draws the menu in MonoCode's `ExplorerMenu` look, in a glass popover;
/// rows report hovers and clicks by index. Every menu of the app's own is
/// drawn here, so they all look alike.
pub fn render_menu(
    view: MenuView<'_>,
    on_hover: impl Fn(usize, &mut gpui::Window, &mut App) + 'static,
    on_pick: impl Fn(usize, &mut gpui::Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut gpui::Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    render_menu_layered(view, MENU_LAYER, on_hover, on_pick, on_dismiss, cx)
}

/// The layer menus draw on, over the window's content.
const MENU_LAYER: usize = 3;

/// [`render_menu`] on its own layer: a menu over something that is
/// itself drawn above menus (the image preview) takes a higher one.
pub fn render_menu_layered(
    view: MenuView<'_>,
    layer: usize,
    on_hover: impl Fn(usize, &mut gpui::Window, &mut App) + 'static,
    on_pick: impl Fn(usize, &mut gpui::Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut gpui::Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let MenuView {
        id,
        entries,
        active,
        place,
        width,
        focus,
        header,
    } = view;
    let theme = cx.theme();
    let colors = &theme.colors;
    let on_hover = std::rc::Rc::new(on_hover);
    let on_pick = std::rc::Rc::new(on_pick);
    let separator = || {
        // `my-1 h-px bg-content/10`
        div().my_1().h(px(1.0)).bg(colors.fg.fill(Fill::Strong))
    };
    let rows = entries.iter().enumerate().map(|(ix, entry)| {
        let MenuEntry::Item(item) = entry else {
            return separator().into_any_element();
        };
        let highlighted = ix == active;
        let red_300: Hsla = rgb(RED_300).into();
        let red_500: Hsla = rgb(RED_500).into();
        let hover_bg = if item.danger {
            red_500.opacity(0.15)
        } else {
            // `hover:bg-content/5`
            colors.fg.fill(Fill::Hover)
        };
        let (hover, pick) = (on_hover.clone(), on_pick.clone());
        div()
            .id(SharedString::from(format!("{id}-row-{ix}")))
            .flex()
            .flex_none()
            .w_full()
            .items_center()
            .gap(px(ROW_GAP))
            .px(px(ROW_PAD_X))
            .rounded(px(ROW_RADIUS))
            .text_size(px(ROW_TEXT))
            .line_height(relative(1.0))
            .map(|el| {
                // `py-1.5` when the row carries a description, else `h-7`.
                if item.description.is_some() {
                    el.py(px(6.0))
                } else {
                    el.h(px(ROW_HEIGHT))
                }
            })
            .map(|el| {
                if item.disabled {
                    // `text-content/30`
                    el.text_color(colors.fg.ink(Ink::Faint))
                } else if item.danger {
                    // `text-red-300/90 hover:bg-red-500/15`, highlighted
                    // `bg-red-500/20 text-red-300`.
                    if highlighted {
                        el.text_color(red_300).bg(red_500.opacity(0.2))
                    } else {
                        el.text_color(red_300.opacity(0.9))
                            .hover(move |s| s.bg(hover_bg))
                    }
                } else {
                    let el = el.text_color(colors.fg);
                    if highlighted {
                        // `bg-selection text-content`
                        el.bg(colors.fg.fill(Fill::Selected))
                    } else {
                        el.hover(move |s| s.bg(hover_bg))
                    }
                }
            })
            .on_hover(move |hovered, window, cx| {
                if *hovered {
                    hover(ix, window, cx);
                }
            })
            .when(!item.disabled, |el| {
                el.cursor_pointer()
                    .on_click(move |_, window, cx| pick(ix, window, cx))
            })
            .when_some(item.tooltip.clone(), |el, tip| {
                el.tooltip(Tooltip::text(tip))
            })
            .children(item.icon.map(|icon| {
                let color = if item.disabled {
                    colors.fg.ink(Ink::Faint)
                } else {
                    colors.fg
                };
                if item.spin {
                    crate::ui::git_changes_panel::spinning_icon(
                        SharedString::from(format!("{id}-icon-{ix}")),
                        icon,
                        IconSize::Sm,
                        color,
                    )
                } else {
                    Icon::new(icon)
                        .size(IconSize::Sm)
                        .color(color)
                        .into_any_element()
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(item.label.clone()))
                    .children(item.description.clone().map(|text| {
                        // `mt-1 text-[11px] leading-snug text-content/50`
                        div()
                            .mt_1()
                            .text_size(Text::Sm.px())
                            .line_height(relative(1.375))
                            .text_color(colors.fg.ink(Ink::Secondary))
                            .child(text)
                    })),
            )
            .when(item.checked, |el| {
                // `Check size-3.5`
                el.child(
                    Icon::new(IconName::Check)
                        .size(IconSize::Sm)
                        .color(colors.fg),
                )
            })
            .when(item.submenu, |el| {
                // `ChevronRight size-3.5 shrink-0 text-content/50`
                el.child(
                    Icon::new(IconName::ChevronRight)
                        .size(IconSize::Sm)
                        .color(colors.fg.ink(Ink::Secondary)),
                )
            })
            .children(item.shortcut.clone().filter(|_| !item.checked).map(|keys| {
                // `shrink-0 text-[11px] text-content/40`
                div()
                    .flex_none()
                    .text_size(Text::Sm.px())
                    .text_color(colors.fg.ink(Ink::Muted))
                    .child(keys)
            }))
            .into_any_element()
    });
    let above = matches!(place, MenuPlace::Above);
    let menu = div()
        .id(id)
        .track_focus(focus)
        // Drawn over other rows: a press or hover here must not reach them.
        .occlude()
        .map(|el| match place {
            MenuPlace::UnderRight(_) => el.min_w(px(width)),
            _ => el.w(px(width)),
        })
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_1()
        .border_color(colors.fg.fill(Fill::Strong))
        .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx))
        // MonoCode `Popover` glass, under everything the menu holds.
        .child(crate::ui::frost::frost(
            crate::ui::frost::POPOVER_RADIUS,
            cx,
        ))
        // The header brings its own `my-1 h-px` separator.
        .children(header)
        .children(rows);
    // MonoCode `Popover` FRAME: `rounded-xl border border-content/10
    // shadow-xl` over a `backdrop-blur-xl` glass tinted `content/2%`
    // (dark) or `background-base` (light), added above; `p-1` content.
    let menu = menu
        .p_1()
        .rounded(px(12.0))
        .shadow_xl()
        // `popover-open`: 170ms `cubic-bezier(0.16, 1, 0.3, 1)` from
        // opacity 0, `scale(0.94)` and 8px towards the anchor
        // (`--popover-lift: -8px` below a point, `8px` above a trigger).
        // GPUI divs cannot scale, so only the fade and the slide are kept.
        .with_animation(
            SharedString::from(format!("{id}-open")),
            gpui::Animation::new(std::time::Duration::from_millis(170))
                .with_easing(gpui::ease_out_quint()),
            move |el, delta| {
                let lift = 8.0 * (1.0 - delta);
                let el = el.opacity(delta);
                el.mt(px(if above { lift } else { -lift }))
            },
        );
    let anchored = match place {
        MenuPlace::At(position) => anchored().position(position),
        MenuPlace::UnderRight(position) => {
            anchored().anchor(gpui::Anchor::TopRight).position(position)
        }
        MenuPlace::Above => anchored()
            .anchor(gpui::Anchor::BottomLeft)
            .offset(gpui::point(px(0.0), px(-4.0))),
        MenuPlace::Below => anchored()
            .anchor(gpui::Anchor::TopLeft)
            .offset(gpui::point(px(0.0), px(4.0))),
    };
    deferred(anchored.snap_to_window().child(menu))
        .with_priority(layer)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<MenuEntry> {
        vec![
            MenuEntry::Item(MenuAction::new("close", "Close Tab")),
            MenuEntry::Separator,
            MenuEntry::Item(MenuAction::new("others", "Close Other Tabs").disabled(true)),
            MenuEntry::Item(MenuAction::new("delete", "Delete").danger()),
        ]
    }

    #[test]
    fn arrows_skip_separators_and_wrap() {
        let e = entries();
        assert_eq!(first_item(&e), 0);
        assert_eq!(step(&e, 0, 1), 2);
        assert_eq!(step(&e, 3, 1), 0);
        assert_eq!(step(&e, 0, -1), 3);
    }

    #[test]
    fn disabled_rows_and_separators_do_not_pick() {
        let e = entries();
        assert_eq!(pick(&e, 0), Some("close"));
        assert_eq!(pick(&e, 1), None);
        assert_eq!(pick(&e, 2), None);
        assert_eq!(pick(&e, 3), Some("delete"));
    }
}
