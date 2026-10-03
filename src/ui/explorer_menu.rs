//! MonoCode `ExplorerMenu`: the context menu used across the app, drawn at
//! the pointer. Rows can carry a description, a shortcut, and a danger tint;
//! ↑/↓ move over the rows (wrapping), Enter or Space picks, Esc closes, and
//! a mouse-down anywhere else dismisses it.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnimationExt, AnyElement, App, InteractiveElement, IntoElement, ParentElement, Pixels, Point,
    SharedString, Styled, anchored, deferred, div, prelude::*, px,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MenuAction {
    pub id: &'static str,
    pub label: String,
    pub description: Option<String>,
    pub shortcut: Option<&'static str>,
    pub disabled: bool,
    pub danger: bool,
    /// MonoCode `checked`: a check at the row's end.
    pub checked: bool,
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
        }
    }

    pub fn description(mut self, text: Option<String>) -> Self {
        self.description = text;
        self
    }

    pub fn shortcut(mut self, keys: &'static str) -> Self {
        self.shortcut = Some(keys);
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

/// Where and what the menu draws.
/// Where the menu opens.
#[derive(Clone, Copy, Debug)]
pub enum MenuPlace {
    /// At a window point, as a context menu does.
    At(Point<Pixels>),
    /// Above the element it is placed in (MonoCode `Popover side="top"`).
    Above,
}

pub struct MenuView<'a> {
    pub id: &'static str,
    pub entries: &'a [MenuEntry],
    pub active: usize,
    pub place: MenuPlace,
    pub width: f32,
    pub focus: &'a gpui::FocusHandle,
}

/// Draws the menu; rows report hovers and clicks by index.
pub fn render_menu(
    view: MenuView<'_>,
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
    } = view;
    let colors = &cx.theme().colors;
    let on_hover = std::rc::Rc::new(on_hover);
    let on_pick = std::rc::Rc::new(on_pick);
    let rows = entries.iter().enumerate().map(|(ix, entry)| {
        let MenuEntry::Item(item) = entry else {
            return div()
                .my_1()
                .h(px(1.0))
                .bg(colors.fg.opacity(0.1))
                .into_any_element();
        };
        let highlighted = ix == active;
        let danger = colors.danger;
        let hover_bg = if item.danger {
            danger.opacity(0.15)
        } else {
            colors.fg.opacity(0.05)
        };
        let (hover, pick) = (on_hover.clone(), on_pick.clone());
        div()
            .id(SharedString::from(format!("{id}-row-{ix}")))
            .flex()
            .w_full()
            .items_center()
            .gap_3()
            .px_2()
            .rounded(px(8.0))
            .text_size(px(13.0))
            .map(|el| {
                if item.description.is_some() {
                    el.py(px(6.0))
                } else {
                    el.h(px(28.0))
                }
            })
            .map(|el| {
                if item.disabled {
                    el.text_color(colors.fg.opacity(0.3))
                } else if item.danger {
                    let el = el.text_color(danger.opacity(0.9));
                    if highlighted {
                        el.bg(danger.opacity(0.2))
                    } else {
                        el.hover(move |s| s.bg(hover_bg))
                    }
                } else {
                    let el = el.text_color(colors.fg);
                    if highlighted {
                        el.bg(colors.active)
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(item.label.clone()))
                    .children(item.description.clone().map(|text| {
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .text_color(colors.fg.opacity(0.5))
                            .child(text)
                    })),
            )
            .when(item.checked, |el| {
                el.child(
                    ely_gpui_component::primitives::Icon::new(
                        ely_gpui_component::primitives::IconName::Check,
                    )
                    .size(ely_gpui_component::theme::IconSize::Xs)
                    .color(colors.fg),
                )
            })
            .children(item.shortcut.filter(|_| !item.checked).map(|keys| {
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(colors.fg.opacity(0.4))
                    .child(keys)
            }))
            .into_any_element()
    });
    // MonoCode `Popover`: rounded-xl frame, and `popover-open` — it fades
    // in rising 8px towards its anchor over 170ms.
    let above = matches!(place, MenuPlace::Above);
    let menu = div()
        .id(id)
        .track_focus(focus)
        .w(px(width))
        .p_1()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface)
        .shadow_xl()
        .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx))
        .children(rows)
        .with_animation(
            SharedString::from(format!("{id}-open")),
            gpui::Animation::new(std::time::Duration::from_millis(170))
                .with_easing(gpui::ease_out_quint()),
            move |el, delta| {
                let lift = px(8.0 * (1.0 - delta));
                let el = el.opacity(delta);
                if above { el.mb(lift) } else { el.mt(lift) }
            },
        );
    let anchored = match place {
        MenuPlace::At(position) => anchored().position(position),
        MenuPlace::Above => anchored()
            .anchor(gpui::Anchor::BottomLeft)
            .offset(gpui::point(px(0.0), px(-4.0))),
    };
    deferred(anchored.snap_to_window().child(menu))
        .with_priority(3)
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
