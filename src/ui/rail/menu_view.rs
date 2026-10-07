//! Drawing MonoCode `TabGroupMenu`: the name field, logo, colours and
//! mascot above the action rows, and the `ExplorerMenu` a row opens
//! beside it.

use std::cell::Cell;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Bounds, Context, Div, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, SharedString, Stateful, Styled, anchored,
    canvas, deferred, div, img, prelude::*, relative, rgb,
};

use crate::ui::scale::px;

use super::model::{self, mute_status, notification_id};
use super::state::{MenuTarget, SubmenuKind};
use super::{group_color, project_name};
use crate::app::BenCodeApp;
use crate::app::session_folders::{FOLDER_COLORS, palette_color, parse_hex, to_hex};
use crate::ui::app_callback::app_callback_with;
use crate::ui::explorer_menu::{self, MenuEntry, MenuPlace, MenuView};
use crate::ui::icons::ExtraIcon;
use crate::ui::mascot::{MASCOT_NAMES, mascot_for, mascot_name_for, pixel_sprite};

/// MonoCode `TabGroupMenu` `MENU_WIDTH`.
const MENU_WIDTH: f32 = 260.0;
/// MonoCode `ExplorerMenu`'s width, for the submenus.
pub(super) const SUBMENU_WIDTH: f32 = 228.0;
/// `ExplorerMenu` metrics: `p-1` in a 1px border, `h-7` rows, `my-1 h-px`
/// separators.
const SUBMENU_INSET: f32 = 5.0;
const SUBMENU_ROW: f32 = 28.0;
const SUBMENU_SEPARATOR: f32 = 9.0;
/// MonoCode `text-red-300` / `text-red-400` / `bg-red-500`.
const RED_300: u32 = 0xfca5a5;
const RED_400: u32 = 0xf87171;
const RED_500: u32 = 0xef4444;

type Anchor = Rc<Cell<Option<Bounds<Pixels>>>>;

/// A menu row's glyph: Ely's, or a Lucide one Ely lacks.
#[derive(Clone, Copy)]
pub(super) enum RowIcon {
    Ely(IconName),
    Extra(ExtraIcon),
}

impl From<IconName> for RowIcon {
    fn from(icon: IconName) -> Self {
        Self::Ely(icon)
    }
}

impl From<ExtraIcon> for RowIcon {
    fn from(icon: ExtraIcon) -> Self {
        Self::Extra(icon)
    }
}

impl RowIcon {
    /// `size-3.5` in `color`.
    fn render(self, color: Hsla) -> AnyElement {
        match self {
            Self::Ely(icon) => Icon::new(icon).size(IconSize::Sm).color(color).into_any_element(),
            Self::Extra(icon) => icon.icon().size(IconSize::Sm).color(color).into_any_element(),
        }
    }
}

/// MonoCode `TabGroupMenuExtraItem`.
pub(super) struct ExtraItem {
    pub id: &'static str,
    pub label: String,
    pub description: Option<String>,
    pub icon: RowIcon,
    pub danger: bool,
    pub sep_before: bool,
    pub disabled: bool,
    pub submenu: Option<SubmenuKind>,
}

impl ExtraItem {
    pub(super) fn new(id: &'static str, label: impl Into<String>, icon: impl Into<RowIcon>) -> Self {
        Self {
            id,
            label: label.into(),
            description: None,
            icon: icon.into(),
            danger: false,
            sep_before: false,
            disabled: false,
            submenu: None,
        }
    }
}

/// Where a submenu of `entries` placed at `position` is drawn.
pub(super) fn submenu_bounds(entries: &[MenuEntry], position: Point<Pixels>) -> Bounds<Pixels> {
    let rows: f32 = entries
        .iter()
        .map(|e| match e {
            MenuEntry::Item(_) => SUBMENU_ROW,
            MenuEntry::Separator => SUBMENU_SEPARATOR,
        })
        .sum();
    Bounds::new(
        position,
        gpui::size(px(SUBMENU_WIDTH), px(rows + SUBMENU_INSET * 2.0)),
    )
}

impl BenCodeApp {
    /// The project or group menu with its open submenu.
    pub(super) fn render_rail_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.rail_ui.menu.as_ref()?;
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let prefs = &self.settings.rail;
        let (extra, color_index, custom, current, mascot_seed, mascot_name, logo_path) = match &menu.target {
            MenuTarget::Project(path) => {
                let key = model::path_key(path);
                (
                    self.project_extra_items(path),
                    prefs.tab_group_colors.get(&key).copied(),
                    prefs.tab_group_custom_colors.get(&key).and_then(|h| parse_hex(h)),
                    self.project_color(path),
                    project_name(path).to_string(),
                    prefs.tab_group_mascots.get(&key).cloned(),
                    Some(prefs.tab_group_logos.get(&key).cloned()),
                )
            }
            MenuTarget::Group(id) => {
                let group = prefs.project_groups.iter().find(|g| &g.id == id)?;
                (
                    Self::group_extra_items(),
                    group.color_index,
                    group.custom_color.as_deref().and_then(parse_hex),
                    group_color(group),
                    group.id.clone(),
                    group.mascot.clone(),
                    None,
                )
            }
        };
        let leading = match &menu.target {
            MenuTarget::Project(path) => mute_status(
                prefs.project_notifications.get(&notification_id(path)),
                crate::app::now_ms(),
            )
            .map(|status| ExtraItem {
                description: Some(status),
                ..ExtraItem::new("notifications-resume", "Resume notifications", ExtraIcon::BellOff)
            }),
            MenuTarget::Group(_) => None,
        };
        let target = menu.target.clone();
        let separator = || div().my_1().h(px(1.0)).bg(fg.opacity(0.1));
        let frame = crate::ui::sidebar_popovers::popover_frame(cx)
            .id("rail-menu")
            .occlude()
            .w(px(MENU_WIDTH))
            .p_2()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                let in_submenu = this.rail_ui.submenu.as_ref().is_some_and(|s| {
                    submenu_bounds(&this.rail_submenu_entries(s.kind), s.position).contains(&event.position)
                });
                if in_submenu || std::mem::take(&mut this.rail_ui.trigger_hit) {
                    return;
                }
                this.close_rail_menu(cx);
            }))
            .children(leading.map(|item| {
                div()
                    .flex()
                    .flex_col()
                    .child(self.render_extra_row(item, cx))
                    .child(separator())
            }))
            .child(self.render_name_field(cx))
            .children(logo_path.map(|logo| {
                let MenuTarget::Project(path) = &target else {
                    return div().into_any_element();
                };
                self.render_logo_row(path, logo, cx).into_any_element()
            }))
            .child(self.render_swatch_row(&target, color_index, custom, cx))
            .when(self.rail_ui.custom_color_open, |el| {
                let target = target.clone();
                el.child(
                    div().mb_2().child(
                        ely_gpui_component::forms::ColorPicker::new("rail-color-picker", custom.unwrap_or(current))
                            .opaque()
                            .on_change(app_callback_with(cx, move |this, color: Hsla, cx| {
                                this.set_rail_custom_color(&target, &to_hex(color), cx);
                            })),
                    ),
                )
            })
            .child(self.render_mascot_row(&target, &mascot_seed, mascot_name.as_deref(), cx))
            .child(separator())
            .children(extra.into_iter().map(|item| {
                let sep = item.sep_before;
                div()
                    .flex()
                    .flex_col()
                    .when(sep, |el| el.child(separator()))
                    .child(self.render_extra_row(item, cx))
            }))
            .children(self.rail_ui.menu_error.clone().map(|err| {
                // `px-2 py-1 text-xs text-red-400`
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(rgb(RED_400))
                    .child(err)
            }));
        Some(
            deferred(anchored().position(menu.position).snap_to_window().child(frame))
                .with_priority(3)
                .into_any_element(),
        )
    }

    pub(super) fn render_rail_submenu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let submenu = self.rail_ui.submenu.as_ref()?;
        let entries = self.rail_submenu_entries(submenu.kind);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "rail-submenu",
                entries: &entries,
                active: submenu.active,
                place: MenuPlace::At(submenu.position),
                width: SUBMENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(s) = this.rail_ui.submenu.as_mut()
                        && s.active != ix
                    {
                        s.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("rail submenu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_rail_submenu(ix, cx)) {
                    log::debug!("rail submenu pick after app drop: {err:#}");
                }
            },
            // A press in the menu it came from closes just the submenu; the
            // menu's own outside check handles presses elsewhere.
            move |_, cx| {
                let closed = close_app.update(cx, |this, cx| {
                    if this.rail_ui.submenu.take().is_some() {
                        cx.notify();
                    }
                });
                if let Err(err) = closed {
                    log::debug!("rail submenu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }

    /// `mb-2 w-full rounded-lg border border-content/10 bg-content/5
    /// px-2.5 py-1.5 text-[13px]`.
    fn render_name_field(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let handle = gpui::Focusable::focus_handle(self.rail_ui.name_input.read(cx), cx);
        div()
            .mb_2()
            .w_full()
            .flex()
            .items_center()
            .rounded(px(8.0))
            .border_1()
            .border_color(fg.opacity(0.1))
            .bg(fg.opacity(0.05))
            .px(px(10.0))
            .py(px(6.0))
            .text_size(px(13.0))
            .text_color(fg)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&handle, cx))
            .child(div().flex_1().min_w_0().child(self.rail_ui.name_input.clone()))
    }

    /// MonoCode's project logo row: `mb-2 flex items-center gap-2 px-0.5`.
    fn render_logo_row(&self, path: &str, logo: Option<String>, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let (pick_path, clear_path) = (path.to_string(), path.to_string());
        let has_logo = logo.is_some();
        div()
            .mb_2()
            .flex()
            .items_center()
            .gap_2()
            .px(px(2.0))
            .child(
                // `grid size-9 rounded-lg border border-content/10 bg-content/5 hover:bg-content/10`
                div()
                    .id("rail-logo-pick")
                    .flex()
                    .flex_none()
                    .size_9()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(fg.opacity(0.1))
                    .bg(fg.opacity(0.05))
                    .hover(move |s| s.bg(fg.opacity(0.1)))
                    .tooltip(ely_gpui_component::primitives::Tooltip::text(if has_logo {
                        "Change project logo"
                    } else {
                        "Add project logo"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_project_logo(&pick_path, cx)))
                    .child(match logo {
                        Some(file) => img(std::path::PathBuf::from(file)).size_5().into_any_element(),
                        None => ExtraIcon::ImagePlus
                            .icon()
                            .size(IconSize::Lg)
                            .color(fg.opacity(0.7))
                            .into_any_element(),
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_size(px(11.0)).text_color(fg.opacity(0.5)).child("Project logo"))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(fg.opacity(0.7))
                            .child(if has_logo {
                                "Shown in tabs and composer"
                            } else {
                                "Optional — replaces folder icon"
                            }),
                    ),
            )
            .when(has_logo, |el| {
                // `grid size-7 rounded-md text-content/50 hover:bg-content/10`
                el.child(
                    div()
                        .id("rail-logo-clear")
                        .group("rail-logo-clear")
                        .flex()
                        .flex_none()
                        .size_7()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .hover(move |s| s.bg(fg.opacity(0.1)))
                        .tooltip(ely_gpui_component::primitives::Tooltip::text("Remove project logo"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_rail_prefs(|prefs| prefs.with_logo(&clear_path, None), cx);
                        }))
                        .child(
                            Icon::new(IconName::Trash2)
                                .size(IconSize::Sm)
                                .color(fg.opacity(0.5))
                                .group_hover_color("rail-logo-clear", fg),
                        ),
                )
            })
    }

    /// MonoCode `ColorSwatchRow`: the palette, then the custom colour.
    fn render_swatch_row(
        &self,
        target: &MenuTarget,
        color_index: Option<usize>,
        custom: Option<Hsla>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let picker_open = self.rail_ui.custom_color_open;
        // `size-3.5 rounded-full`, `ring-2 ring-content/80 ring-offset-1` when picked.
        let swatch = |id: SharedString, color: Option<Hsla>, selected: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .size_5()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(if selected { 18.0 } else { 14.0 }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .when(selected, |el| el.border_2().border_color(fg.opacity(0.8)))
                        .child(
                            div()
                                .size(px(14.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .map(|el| match color {
                                    Some(color) => el.bg(color),
                                    None => el.border_1().border_color(fg.opacity(0.3)),
                                }),
                        ),
                )
        };
        let mut row = div().mb_2().flex().items_center().justify_between().gap_1().px(px(2.0));
        for ix in 0..FOLDER_COLORS.len() {
            let selected = custom.is_none() && (color_index == Some(ix) || (color_index.is_none() && ix == 0));
            let target = target.clone();
            row = row.child(
                swatch(SharedString::from(format!("rail-color-{ix}")), palette_color(ix), selected)
                    .tooltip(ely_gpui_component::primitives::Tooltip::text(format!("Color {}", ix + 1)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.rail_ui.custom_color_open = false;
                        this.set_rail_color(&target, (ix > 0).then_some(ix), cx);
                    })),
            );
        }
        let pipette = custom.is_some() || picker_open;
        row.child(
            swatch(SharedString::from("rail-color-custom"), custom, pipette)
                .tooltip(ely_gpui_component::primitives::Tooltip::text("Custom color"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.rail_ui.custom_color_open = !this.rail_ui.custom_color_open;
                    cx.notify();
                }))
                .when(custom.is_none(), |el| {
                    el.child(
                        div()
                            .absolute()
                            .child(Icon::new(IconName::Pipette).size(IconSize::Xs).color(fg.opacity(0.6))),
                    )
                }),
        )
    }

    /// MonoCode's "Mascot" row: `mb-1 text-[11px] text-content/50`, then a
    /// `size-5 rounded-md` swatch per mascot.
    fn render_mascot_row(
        &self,
        target: &MenuTarget,
        seed: &str,
        name: Option<&str>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let shown = mascot_name_for(seed, name);
        // `bg-selection-hover ring-1 ring-content/50`
        let selected_bg = fg.opacity(if theme.is_dark() { 0.14 } else { 0.08 });
        div()
            .mb_2()
            .px(px(2.0))
            .child(div().mb_1().text_size(px(11.0)).text_color(fg.opacity(0.5)).child("Mascot"))
            .child(div().flex().items_center().justify_between().gap_1().children(
                MASCOT_NAMES.iter().map(|mascot| {
                    let selected = *mascot == shown;
                    let target = target.clone();
                    let sprite = mascot_for(seed, Some(mascot));
                    div()
                        .id(SharedString::from(format!("rail-mascot-{mascot}")))
                        .flex()
                        .flex_none()
                        .size_5()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .map(|el| {
                            if selected {
                                el.bg(selected_bg).border_1().border_color(fg.opacity(0.5))
                            } else {
                                el.hover(move |s| s.bg(fg.opacity(0.08)))
                            }
                        })
                        .tooltip(ely_gpui_component::primitives::Tooltip::text(*mascot))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_rail_mascot(&target, mascot, cx)))
                        .child(pixel_sprite(&sprite.rest, px(12.0), fg.opacity(0.75), false))
                }),
            ))
    }

    /// MonoCode `MenuRow`: `flex min-h-8 items-center gap-2.5 rounded-lg
    /// px-2 text-[13px] leading-none`, a `size-3.5` icon at 55%.
    fn render_extra_row(&self, item: ExtraItem, cx: &Context<Self>) -> Stateful<Div> {
        let fg = cx.theme().colors.fg;
        let (red_300, red_500): (Hsla, Hsla) = (rgb(RED_300).into(), rgb(RED_500).into());
        let anchor: Anchor = Rc::default();
        let expanded = item
            .submenu
            .is_some_and(|kind| self.rail_ui.submenu.as_ref().is_some_and(|s| s.kind == kind));
        let id = item.id;
        let (submenu, disabled) = (item.submenu, item.disabled);
        let hover_anchor = anchor.clone();
        let click_anchor = anchor.clone();
        let hover_bg = if item.danger { red_500.opacity(0.15) } else { fg.opacity(0.05) };
        div()
            .id(SharedString::from(format!("rail-menu-{id}")))
            .relative()
            .flex()
            .w_full()
            .min_h(px(32.0))
            .items_center()
            .gap(px(10.0))
            .rounded(px(8.0))
            .px_2()
            .text_size(px(13.0))
            .line_height(relative(1.0))
            .map(|el| {
                if disabled {
                    el.text_color(fg.opacity(0.3))
                } else if item.danger {
                    el.text_color(red_300.opacity(0.9)).hover(move |s| s.bg(hover_bg))
                } else {
                    el.text_color(fg)
                        .when(expanded, |el| el.bg(hover_bg))
                        .hover(move |s| s.bg(hover_bg))
                }
            })
            .child(canvas(move |bounds, _, _| anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if !*hovered {
                    return;
                }
                match (submenu, hover_anchor.get()) {
                    (Some(kind), Some(bounds)) if !disabled => {
                        this.open_rail_submenu(kind, submenu_origin(bounds), cx)
                    }
                    _ => {
                        if this.rail_ui.submenu.take().is_some() {
                            cx.notify();
                        }
                    }
                }
            }))
            .when(!disabled, |el| {
                el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_rail_extra(id, click_anchor.get(), cx);
                }))
            })
            .child(item.icon.render(fg.opacity(0.55)))
            .child(
                // `min-w-0 flex-1 leading-label`, `py-2` with a description.
                div()
                    .min_w_0()
                    .flex_1()
                    .line_height(relative(1.4))
                    .when(item.description.is_some(), |el| el.py_2())
                    .child(div().truncate().child(item.label))
                    .children(item.description.map(|text| {
                        // `mt-1 block text-[11px] leading-snug text-content/60`
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .line_height(relative(1.375))
                            .text_color(fg.opacity(0.6))
                            .child(text)
                    })),
            )
            .when(submenu.is_some(), |el| {
                el.child(Icon::new(IconName::ChevronRight).size(IconSize::Sm).color(fg.opacity(0.5)))
            })
    }

}

/// A submenu's origin beside its row: MonoCode `ExplorerMenu` anchored to
/// the row's right edge, its first row level with it.
pub(super) fn submenu_origin(row: Bounds<Pixels>) -> Point<Pixels> {
    Point::new(row.right() + px(4.0), row.top() - px(SUBMENU_INSET))
}
