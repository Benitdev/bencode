use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Entity, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Role, Stateful, Styled, Window, canvas, div, prelude::*,
};

use super::{
    aim,
    menu::{Open, at_depth, choose},
    model::{Entry, Kind, Menu, MenuItem},
};
use crate::{
    forms::{Run, surface},
    navigation::marked,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, ControlSize, IconSize, Radius},
    typography::KbdCombo,
};

fn row(item: &MenuItem, ix: usize, current: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let colors = &theme.colors;
    let fg = if item.disabled {
        colors.fg_disabled
    } else {
        colors.fg
    };
    let (lead, lead_color) = match &item.kind {
        Kind::Check(true) => (Some(IconName::Check), fg),
        _ => (item.icon, if item.disabled { fg } else { colors.fg_muted }),
    };
    div()
        .id(("row", ix))
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .h(theme.control_height(ControlSize::Md))
        .px_2()
        .rounded(theme.radius(Radius::Md))
        .text_color(fg)
        .when(current && !item.disabled, |row| row.bg(colors.hover))
        .when(!item.disabled, |row| row.cursor_pointer())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(theme.icon_size(IconSize::Sm))
                .when_some(lead, |slot, icon| {
                    slot.child(Icon::new(icon).size(IconSize::Sm).color(lead_color))
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(if item.hits.is_empty() {
                    item.label.clone().into_any_element()
                } else {
                    marked(item.label.clone(), item.hits.clone(), cx)
                }),
        )
        .when_some(item.keys.clone(), |row, keys| {
            row.child(
                div()
                    .flex_none()
                    .pl_4()
                    .text_color(colors.fg_subtle)
                    .child(KbdCombo::new(&keys)),
            )
        })
        .when(matches!(item.kind, Kind::Sub(_)), |row| {
            row.child(
                Icon::new(IconName::ChevronRight)
                    .size(IconSize::Xs)
                    .color(colors.fg_subtle),
            )
        })
}

pub(super) fn panel(
    id: &ElementId,
    menu: &Menu,
    state: &Entity<Open>,
    depth: usize,
    close: &Run,
    head: Option<AnyElement>,
    window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let open = state.read(cx);
    let here = at_depth(menu, &open.levels, depth).expect("settled levels follow submenu rows");
    let at = open.levels[depth].at;
    let theme = cx.theme();
    let colors = &theme.colors;
    let rows = here
        .entries
        .iter()
        .enumerate()
        .map(|(ix, entry)| match entry {
            Entry::Separator => div().h_px().my_1().bg(colors.border).into_any_element(),
            Entry::Item(item) => {
                let marked = at == Some(ix);
                let sub = matches!(item.kind, Kind::Sub(_));
                let (hover, click, close, menu) =
                    (state.clone(), state.clone(), close.clone(), menu.clone());
                let hover_menu = menu.clone();
                let label = item.label.clone();
                row(item, ix, marked, cx)
                    .debug_selector(move || format!("menu-row-{label}"))
                    .when(!item.disabled, |row| {
                        let left = state.clone();
                        row.on_mouse_move(move |event, _, cx| {
                            aim::hover(&hover, &hover_menu, (depth, ix), sub, event.position, cx)
                        })
                        .on_hover(move |hovered, _, cx| {
                            if !hovered {
                                aim::leave(&left, (depth, ix), cx);
                            }
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                choose(&click, &menu, depth, ix, &close, window, cx);
                            },
                        )
                    })
                    .when(marked && sub, |row| {
                        row.child(measure(state.clone(), move |open, bounds| {
                            let level = &mut open.levels[depth];
                            let changed = level.row != bounds;
                            level.row = bounds;
                            changed
                        }))
                    })
                    .into_any_element()
            }
        });
    surface((id.clone(), format!("panel-{depth}")), window, cx)
        .role(Role::Menu)
        .debug_selector(move || format!("menu-panel-{depth}"))
        .relative()
        // Rows under this panel take no hover.
        .occlude()
        // Presses here are the menu's, not its host's.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .min_w(theme.menu_width())
        .p_1()
        .flex()
        .flex_col()
        .children(head)
        .children(rows)
        .child(measure(state.clone(), move |open, bounds| {
            let level = &mut open.levels[depth];
            let changed = level.panel != bounds;
            level.panel = bounds;
            changed
        }))
}

/// Records the host's, a marked row's or a panel's box.
pub(super) fn measure(
    state: Entity<Open>,
    write: impl Fn(&mut Open, Bounds<Pixels>) -> bool + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            if state.update(cx, |open, _| write(open, bounds)) {
                state.update(cx, |_, cx| cx.notify());
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
