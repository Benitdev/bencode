//! MonoCode `MonoSettingsDialog` (`size="lg" minimalHeader fitViewport`),
//! with `MonoFilePages`' `SoulPage` and `MemoryPage`, `MonoProjects` and
//! `ConfirmReset`: who a Mono is and how it works, in a dialog of its own.
//! A nav of General, Soul and Memory on the left, the section on the right.
//!
//! General: the face (opening its mascots and colours), the name edited in
//! place, the Model and its settings, Permissions, its projects, then
//! "Sessions it starts" (in the sidebar, in a folder), then Reset
//! conversation and Delete. Soul: `SOUL.md` as its
//! markdown source, saved when the field is left, with "Use theirs / Keep
//! mine" when it changed on disk meanwhile. Memory: one fact per row,
//! added, reworded and forgotten in place, over the gauge of what loads.
//!
//! The dialog is drawn in an Ely layer (`Backdrop`), so what opens inside
//! it is raised in that layer, never deferred.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::forms::{Choice, Select};
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Backdrop, Icon, IconName, Tooltip, raise};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::app::mono_files::MemoryEdit;
use crate::app::monos::SettingsSection;
use crate::monos::memory::{MemoryLine, memory_lines, memory_within_budget};
use crate::monos::{MONO_COLOR_COUNT, MONO_COLOR_LABELS, Mono, mono_palette_color};
use crate::ui::app_callback::app_callback;
use crate::ui::mascot::{MASCOT_NAMES, MascotLook, mascot_for, pixel_mascot};
use crate::ui::scale::px;
use crate::ui::settings_parts::{SettingsGroup, SettingsRow};
use crate::ui::sidebar_popovers::pretty_path;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `w-[min(880px,…)]` and `h-[min(680px,…)]`.
const DIALOG_WIDTH: f32 = 880.0;
const DIALOG_HEIGHT: f32 = 680.0;
/// `w-48`.
const NAV_WIDTH: f32 = 192.0;
/// `w-56`: the control column in a Mono's settings row.
const CONTROL_WIDTH: f32 = 224.0;

const SECTIONS: [(SettingsSection, &str, IconName); 3] = [
    (SettingsSection::General, "General", IconName::Settings),
    (SettingsSection::Soul, "Soul", IconName::Sparkles),
    (SettingsSection::Memory, "Memory", IconName::StickyNote),
];

/// MonoCode `shortDate`: "Oct 4".
pub(crate) fn short_date(date: &str) -> String {
    jiff::civil::Date::strptime("%Y-%m-%d", date)
        .map(|d| d.strftime("%b %-d").to_string())
        .unwrap_or_else(|_| date.to_string())
}

/// MonoCode `PageHeader` heading a section: `flex h-10 items-center
/// border-b border-stroke pl-4`, room on the right for the close button.
fn page_header(title: &'static str, action: Option<AnyElement>, fg: Hsla) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .h(px(40.0))
        .pl_4()
        .pr(px(12.0 + 32.0))
        .border_b_1()
        .border_color(fg.fill(Fill::Line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(Text::Md.px())
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg)
                .child(title),
        )
        .children(action)
}

/// MonoCode `Empty`: `px-2 py-6 text-center text-[12px] text-content/45`.
fn empty(text: &'static str, fg: Hsla) -> gpui::Div {
    div()
        .px_2()
        .py_6()
        .flex()
        .justify_center()
        .text_size(Text::Base.px())
        .text_color(fg.ink(Ink::Muted))
        .child(text)
}

/// A small text button: `rounded-md border border-stroke px-3 py-1.5
/// text-[12px]`, red for what deletes.
fn outline_button(
    id: &'static str,
    label: &'static str,
    danger: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    let ink = if danger {
        colors.danger
    } else {
        fg.ink(Ink::Strong)
    };
    let hover = if danger {
        colors.danger.fill(Fill::Strong)
    } else {
        fg.fill(Fill::Hover)
    };
    div()
        .id(id)
        .flex_none()
        .rounded(px(6.0))
        .border_1()
        .border_color(fg.fill(Fill::Line))
        .px_3()
        .py_1p5()
        .text_size(Text::Base.px())
        .text_color(ink)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
}

impl BenCodeApp {
    /// The dialog, while `monos.settings_open` is set over a Mono in view.
    /// A full-window dialog: in `BenCodeApp::render`'s `dialogs` list.
    pub(crate) fn render_mono_settings(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.monos.settings_open || !self.mono_covers() {
            return None;
        }
        let mono = self.mono_in_view()?.clone();
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let section = self.monos.settings_section;
        let nav =
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(1.0))
                .w(px(NAV_WIDTH))
                .h_full()
                .p_2()
                .border_r_1()
                .border_color(fg.fill(Fill::Line))
                .child(
                    div()
                        .px(px(10.0))
                        .pb_2()
                        .pt_1p5()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child("Settings"),
                )
                .children(SECTIONS.iter().map(|&(id, label, icon)| {
                    let selected = id == section;
                    let ink = if selected { fg } else { fg.ink(Ink::Body) };
                    div()
                        .id(SharedString::from(format!("mono-settings-nav-{label}")))
                        .group(SharedString::from(format!("mono-settings-nav-{label}")))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .rounded(px(8.0))
                        .px(px(10.0))
                        .py_1p5()
                        .text_size(Text::Md.px())
                        .text_color(ink)
                        .cursor_pointer()
                        .map(|el| {
                            if selected {
                                el.bg(fg.fill(Fill::Selected))
                            } else {
                                el.hover(move |s| s.bg(fg.fill(Fill::Hover)).text_color(fg))
                            }
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_mono_settings_section(id, cx)
                        }))
                        .child(
                            Icon::new(icon)
                                .size(IconSize::Sm)
                                .color(ink)
                                .group_hover_color(format!("mono-settings-nav-{label}"), fg),
                        )
                        .child(label)
                }));
        let body = match section {
            SettingsSection::General => self.render_mono_general(&mono, cx),
            SettingsSection::Soul => self.render_mono_soul_page(&mono, cx),
            SettingsSection::Memory => self.render_mono_memory_page(&mono, cx),
        };
        let close = app_callback(cx, |this, cx| {
            this.close_mono_settings(cx);
        });
        let close_button = IconButton::new("mono-settings-close", IconName::X)
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .tooltip("Close")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_mono_settings(cx);
            }));
        let card = div()
            .id("mono-settings")
            .relative()
            .flex()
            .w(px(DIALOG_WIDTH))
            .max_w(relative(0.96))
            .h(px(DIALOG_HEIGHT))
            .max_h(relative(0.94))
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(fg.fill(Fill::Line))
            .bg(colors.bg)
            .shadow_2xl()
            .text_color(fg)
            .child(nav)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(body),
            )
            .child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(8.0))
                    .child(close_button),
            );
        Some(
            Backdrop::new("mono-settings-scrim")
                .on_dismiss(close)
                .child(card)
                .into_any_element(),
        )
    }

    /// MonoCode `GeneralSection`: `px-8 pb-8 pt-6`, scrolling.
    fn render_mono_general(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let face = self.render_mono_look_button(mono, cx);
        let name = div()
            .id("mono-settings-name")
            .mt_3()
            .w_full()
            .max_w(px(256.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .px_2()
            .rounded(px(6.0))
            .hover(move |s| s.bg(fg.fill(Fill::Hover)))
            .text_size(Text::Xl.px())
            .font_weight(FontWeight::MEDIUM)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.monos.files.name_input.clone()),
            );
        let session = self
            .mono_view_session()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id));
        let mut model_rows = Vec::new();
        if let Some(session) = session {
            model_rows.push(
                SettingsRow::new("Model")
                    .description("What it thinks and replies with.")
                    .control_width(CONTROL_WIDTH)
                    .control(self.render_mono_settings_model(session, cx))
                    .into_any_element(),
            );
            for (label, control) in self.mono_settings_controls(session, cx) {
                model_rows.push(
                    SettingsRow::new(label)
                        .control_width(CONTROL_WIDTH)
                        .control(control)
                        .into_any_element(),
                );
            }
            let mode = self.session_permission_mode(Some(session));
            let weak = cx.entity().downgrade();
            let picker = Select::new(
                "mono-settings-permissions",
                crate::ui::composer::permission_modes()
                    .iter()
                    .map(|(mode, label, hint, icon)| {
                        Choice::new(mode.id(), *label).icon(*icon).note(*hint)
                    }),
            )
            .label("Permissions")
            .size(ControlSize::Sm)
            .selected(mode.id())
            .on_change(move |value, _, cx| {
                let Some(mode) = crate::app::PermissionMode::from_id(value) else {
                    return;
                };
                if let Err(err) = weak.update(cx, |this, cx| this.set_mono_runtime_mode(mode, cx)) {
                    log::debug!("mono permissions after app drop: {err:#}");
                }
            });
            model_rows.push(
                SettingsRow::new("Permissions")
                    .description("What it may do without asking.")
                    .control_width(CONTROL_WIDTH)
                    .control(picker)
                    .into_any_element(),
            );
        }
        let reset = SettingsRow::new("Reset conversation")
            .description("Clear all messages and start fresh. Its soul, memory and habits stay.")
            .control_width(CONTROL_WIDTH)
            .control(
                outline_button("mono-settings-reset", "Reset", true, cx).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.monos.pending_reset = true;
                        cx.notify();
                    },
                )),
            )
            .into_any_element();
        let delete_id = mono.id.clone();
        let delete = SettingsRow::new("Delete mono")
            .description("Its conversation goes with it. Its soul and memory stay on disk.")
            .control_width(CONTROL_WIDTH)
            .control(
                outline_button("mono-settings-delete", "Delete", true, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.monos.pending_delete = Some(delete_id.clone());
                        cx.notify();
                    },
                )),
            )
            .into_any_element();
        let groups = div()
            .flex()
            .flex_col()
            .gap_7()
            .pt_7()
            .when(!model_rows.is_empty(), |el| {
                el.child(SettingsGroup::new("Model").rows(model_rows))
            })
            .child(
                SettingsGroup::new("Projects")
                    .description("It can see every session, note and folder in these.")
                    .rows(self.render_mono_settings_projects(mono, cx)),
            )
            .child(
                SettingsGroup::new("Sessions it starts")
                    .rows(self.render_mono_started_sessions(mono, cx)),
            )
            .child(SettingsGroup::new("Conversation").rows(vec![reset, delete]));
        let page = div()
            .id("mono-settings-general")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .pb_8()
            .pt_6()
            .child(
                div()
                    .text_size(Text::Lg.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child("General"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pb_6()
                    .pt_4()
                    .child(face)
                    .child(name),
            )
            .child(groups)
            .children(self.monos.reset_error.clone().map(|err| {
                div()
                    .pt_2()
                    .px_1()
                    .text_size(Text::Base.px())
                    .text_color(colors.danger)
                    .child(err)
            }));
        crate::ui::scrollbar::Scrolled::new("mono-settings-general-scrollbar", page)
            .into_any_element()
    }

    /// MonoCode's "Sessions it starts": shown in the project sidebar, and
    /// filed in a folder named after it (only while shown).
    fn render_mono_started_sessions(&self, mono: &Mono, cx: &Context<Self>) -> Vec<AnyElement> {
        let shown = mono.show_started_sessions_in_sidebar;
        let switch = |id: String, on: bool, set: fn(&mut Mono, bool)| {
            let (entity, mono_id) = (cx.entity().downgrade(), mono.id.clone());
            ely_gpui_component::forms::Switch::new(SharedString::from(id), on).on_change(
                move |on, _, cx| {
                    let mono_id = mono_id.clone();
                    let toggled = entity.update(cx, |this, cx| {
                        this.update_monos(
                            |roster| {
                                if let Some(entry) = roster.iter_mut().find(|m| m.id == mono_id) {
                                    set(entry, on);
                                }
                            },
                            cx,
                        );
                        this.play_cue(crate::sounds::Cue::Switch);
                    });
                    if let Err(err) = toggled {
                        log::debug!("Mono switch after app drop: {err:#}");
                    }
                },
            )
        };
        vec![
            SettingsRow::new("Show sessions in the navigator")
                .description(
                    "Sessions it starts appear among your project's threads. Hidden ones stay saved and open from its overview.",
                )
                .control_width(CONTROL_WIDTH)
                .control(switch(format!("mono-started-shown-{}", mono.id), shown, |m, on| {
                    m.show_started_sessions_in_sidebar = on
                }))
                .into_any_element(),
            SettingsRow::new("Group in a folder")
                .control_width(CONTROL_WIDTH)
                .control(
                    switch(
                        format!("mono-started-folder-{}", mono.id),
                        shown && mono.use_sidebar_folders,
                        |m, on| m.use_sidebar_folders = on,
                    )
                    .disabled(!shown),
                )
                .into_any_element(),
        ]
    }

    /// MonoCode `LookButton`: the face (`size-20 rounded-2xl border`, the
    /// mascot at `size-12`), opening its mascots and colours below it.
    fn render_mono_look_button(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let open = self.monos.look_open;
        let color = self.mono_color(mono);
        let button = div()
            .id("mono-settings-look")
            .flex()
            .flex_none()
            .size(px(80.0))
            .items_center()
            .justify_center()
            .rounded(px(16.0))
            .border_1()
            .border_color(fg.fill(Fill::Line))
            .bg(fg.fill(if open { Fill::Selected } else { Fill::Hover }))
            .hover(move |s| s.bg(fg.fill(Fill::Selected)))
            .cursor_pointer()
            .tooltip(Tooltip::text("Change mascot and color"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.monos.look_open = !this.monos.look_open;
                cx.notify();
            }))
            .child(pixel_mascot(
                self.mono_mascot(mono),
                48.0,
                color,
                MascotLook::idle(&mono.mascot),
                cx,
            ));
        let popover = open.then(|| {
            // `width={272}` centred under the 80px face.
            let card = div()
                .id("mono-settings-look-popover")
                .occlude()
                .absolute()
                .top(px(88.0))
                .left(px((80.0 - 272.0) / 2.0))
                .w(px(272.0))
                .p_3()
                .rounded(px(12.0))
                .border_1()
                .border_color(cx.theme().colors.border)
                .bg(crate::ui::sidebar_popovers::popover_glass(cx))
                .shadow_xl()
                .child(self.render_mono_mascot_grid(mono, cx))
                .child(div().my_3().h(px(1.0)).bg(fg.fill(Fill::Strong)))
                .child(self.render_mono_color_grid(mono, cx));
            raise("mono-settings-look-layer", card).with_priority(1)
        });
        div()
            .relative()
            .child(button)
            .children(popover)
            .into_any_element()
    }

    /// MonoCode `MascotPicker columns={5}`: `grid-cols-5 gap-1`, `h-12
    /// rounded-lg` tiles, the mascot at `size-8`.
    fn render_mono_mascot_grid(&self, mono: &Mono, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let color = self.mono_color(mono);
        let selected_bg = fg.opacity(if theme.is_dark() { 0.1 } else { 0.06 });
        let tile = |name: &'static str| {
            let selected = name == mono.mascot;
            let id = mono.id.clone();
            let mut title = name.to_string();
            if let Some(first) = title.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            div()
                .id(SharedString::from(format!("mono-mascot-{name}")))
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(48.0))
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .map(|el| {
                    if selected {
                        el.bg(selected_bg).border_1().border_color(fg.opacity(0.2))
                    } else {
                        el.hover(move |s| s.bg(fg.fill(Fill::Hover)))
                    }
                })
                .tooltip(Tooltip::text(title))
                .on_click(cx.listener(move |this, _, _, cx| this.set_mono_mascot(&id, name, cx)))
                .child(crate::ui::mascot::pixel_sprite(
                    &mascot_for(name, Some(name)).rest,
                    px(32.0),
                    color,
                    false,
                ))
        };
        div().children(MASCOT_NAMES.chunks(5).enumerate().map(|(row, names)| {
            div()
                .flex()
                .gap_1()
                .when(row > 0, |el| el.mt_1())
                .children(names.iter().map(|name| tile(name)))
        }))
    }

    /// MonoCode `ColorPicker columns={5}`: the nine presets (the project
    /// palette and Indigo) as large swatches, the picked one ringed. A
    /// custom colour is set from the rail's menu.
    fn render_mono_color_grid(&self, mono: &Mono, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let swatch = |index: usize| {
            let selected = mono.custom_color.is_none() && mono.color_index == index;
            let id = mono.id.clone();
            div()
                .id(("mono-color", index))
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(32.0))
                .items_center()
                .justify_center()
                .cursor_pointer()
                .tooltip(Tooltip::text(MONO_COLOR_LABELS[index - 1]))
                .on_click(cx.listener(move |this, _, _, cx| this.set_mono_color(&id, index, cx)))
                .child(
                    div()
                        .flex()
                        .size(px(26.0))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .when(selected, |el| {
                            el.border_2().border_color(fg.ink(Ink::Strong))
                        })
                        .child(
                            div()
                                .size(px(if selected { 16.0 } else { 22.0 }))
                                .rounded_full()
                                .bg(mono_palette_color(index).unwrap_or_default()),
                        ),
                )
        };
        let indices: Vec<usize> = (1..=MONO_COLOR_COUNT).collect();
        div().children(indices.chunks(5).enumerate().map(|(row, chunk)| {
            let filler = 5 - chunk.len();
            div()
                .flex()
                .gap(px(2.0))
                .when(row > 0, |el| el.mt_2())
                .children(chunk.iter().map(|ix| swatch(*ix).into_any_element()))
                .children((0..filler).map(|_| div().flex_1().into_any_element()))
        }))
    }

    /// MonoCode `MonoProjects`: one row each (its face, name and folder,
    /// Remove on hover), then Add project.
    fn render_mono_settings_projects(&self, mono: &Mono, cx: &Context<Self>) -> Vec<AnyElement> {
        let fg = cx.theme().colors.fg;
        let mut rows: Vec<AnyElement> = self
            .mono_projects(mono)
            .into_iter()
            .enumerate()
            .map(|(ix, (name, path))| {
                let group = SharedString::from(format!("mono-settings-project-{ix}"));
                let (id, target) = (mono.id.clone(), path.clone());
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .flex()
                    .min_h(px(48.0))
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .size(px(32.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(8.0))
                            .bg(fg.fill(Fill::Hover))
                            .child(crate::ui::mascot::pixel_sprite(
                                &self.project_mascot(&path).rest,
                                px(16.0),
                                self.project_color(&path),
                                false,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(Text::Md.px())
                                    .line_height(px(20.0))
                                    .text_color(fg.ink(Ink::Strong))
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(Text::Sm.px())
                                    .line_height(px(16.0))
                                    .text_color(fg.ink(Ink::Muted))
                                    .child(pretty_path(&path)),
                            ),
                    )
                    .child(
                        div()
                            .id(("mono-settings-project-remove", ix))
                            .flex()
                            .flex_none()
                            .size(px(24.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                            .tooltip(Tooltip::text(format!("Remove {name}")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_mono_project(&id, &target, cx)
                            }))
                            .child(
                                Icon::new(IconName::X)
                                    .size(IconSize::Sm)
                                    .color(fg.ink(Ink::Muted)),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let choices = self.mono_project_choices(mono);
        let mut menu = Menu::new();
        if choices.is_empty() {
            menu = menu.item(MenuItem::new("It already works on every project.").disabled(true));
        }
        for path in choices {
            let (id, target) = (mono.id.clone(), path.clone());
            menu = menu.item(
                MenuItem::new(self.rail_project_label(&path))
                    .icon(IconName::Folder)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.add_mono_project(&id, &target, cx)
                    })),
            );
        }
        let pick_id = mono.id.clone();
        let menu = menu.separator().item(
            MenuItem::new("Choose folder…")
                .icon(IconName::FolderPlus)
                .on_click(app_callback(cx, move |this, cx| {
                    this.pick_mono_project(&pick_id, cx)
                })),
        );
        rows.push(
            div()
                .flex()
                .h(px(44.0))
                .items_center()
                .px_2()
                .child(
                    DropdownMenu::new("mono-settings-add-project", "Add project", menu)
                        .icon(IconName::Plus)
                        .variant(ButtonVariant::Ghost),
                )
                .into_any_element(),
        );
        rows
    }

    /// MonoCode `SoulPage`: the soul as its markdown source, in a field that
    /// saves when it is left (`FileField`).
    fn render_mono_soul_page(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let files = &self.monos.files;
        let loaded = self.mono_files(&mono.id).is_some();
        let field = if !loaded {
            empty("Loading…", fg).into_any_element()
        } else {
            div()
                .px_4()
                .py_3()
                .font_family(cx.theme().mono_family.clone())
                .text_size(Text::Base.px())
                .line_height(px(19.0))
                .child(files.soul_input.clone())
                .into_any_element()
        };
        let conflict = files.soul_conflict.then(|| {
            let link = |id: &'static str, label: &'static str| {
                div()
                    .id(id)
                    .rounded(px(4.0))
                    .px_1p5()
                    .py_0p5()
                    .cursor_pointer()
                    .text_color(fg.ink(Ink::Body))
                    .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                    .child(label)
            };
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .mt_2()
                .px_4()
                .text_size(Text::Sm.px())
                .line_height(px(16.0))
                .text_color(fg.ink(Ink::Secondary))
                .child(div().flex_1().min_w_0().child(format!(
                    "{} changed this while you were editing.",
                    mono.display_name()
                )))
                .child(
                    link("mono-soul-theirs", "Use theirs")
                        .on_click(cx.listener(|this, _, _, cx| this.use_their_soul(cx))),
                )
                .child(
                    link("mono-soul-mine", "Keep mine")
                        .on_click(cx.listener(|this, _, _, cx| this.save_mono_soul(true, cx))),
                )
        });
        let error = files
            .soul_error
            .clone()
            .or_else(|| files.load_error.clone())
            .map(|err| {
                div()
                    .px_4()
                    .pt_2()
                    .text_size(Text::Base.px())
                    .text_color(colors.danger)
                    .child(err)
            });
        let body = div()
            .id("mono-settings-soul")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_4()
            .child(field)
            .children(conflict)
            .children(error);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(page_header("Soul", None, fg))
            .child(crate::ui::scrollbar::Scrolled::new(
                "mono-settings-soul-scrollbar",
                body,
            ))
            .into_any_element()
    }

    /// MonoCode `MemoryPage`: what the Mono remembers, one fact per row;
    /// click a fact to reword it. The gauge of what loads at its foot.
    fn render_mono_memory_page(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let state = &self.monos.files;
        let files = self.mono_files(&mono.id);
        let adding = state.memory_edit == Some(MemoryEdit::Adding);
        let add = files.map(|_| {
            IconButton::new("mono-memory-add", IconName::Plus)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip("Add memory")
                .disabled(adding)
                .on_click(cx.listener(|this, _, window, cx| this.start_adding_memory(window, cx)))
                .into_any_element()
        });
        let editing = match &state.memory_edit {
            Some(MemoryEdit::Editing(line)) => Some(line.clone()),
            _ => None,
        };
        let list: AnyElement = match files {
            None => empty("Loading…", fg).into_any_element(),
            Some(files) => {
                let mut facts = memory_lines(&files.memory);
                // The editor keeps its draft even if a reload took its row.
                if let Some(edit) = &editing
                    && !facts.iter().any(|fact| fact.line == edit.line)
                {
                    let at = edit.index.min(facts.len());
                    facts.insert(at, edit.clone());
                }
                if facts.is_empty() && !adding {
                    empty("Nothing remembered yet", fg).into_any_element()
                } else {
                    div()
                        .when(adding, |el| el.child(self.render_fact_editor(cx)))
                        .children(facts.into_iter().enumerate().map(|(ix, fact)| {
                            if editing.as_ref().is_some_and(|edit| edit.line == fact.line) {
                                self.render_fact_editor(cx)
                            } else {
                                self.render_fact_row(ix, fact, cx)
                            }
                        }))
                        .into_any_element()
                }
            }
        };
        let gauge = files.map(|files| memory_gauge(&files.memory, cx));
        let body = div()
            .id("mono-settings-memory")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .child(list);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(page_header("Memory", add, fg))
            .children(state.memory_error.clone().map(|err| {
                div()
                    .px_4()
                    .py_2()
                    .text_size(Text::Base.px())
                    .text_color(colors.danger)
                    .child(err)
            }))
            .child(crate::ui::scrollbar::Scrolled::new(
                "mono-settings-memory-scrollbar",
                body,
            ))
            .children(gauge.map(|gauge| {
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(fg.fill(Fill::Line))
                    .px_2()
                    .pb_3()
                    .child(gauge)
            }))
            .into_any_element()
    }

    /// MonoCode `FactRow`: its icon, the fact and when it was noted, Edit
    /// and Forget on hover. A struck fact is kept for the record only.
    fn render_fact_row(&self, ix: usize, fact: MemoryLine, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let group = SharedString::from(format!("mono-fact-{ix}"));
        let struck = fact.struck;
        let action = |id: (&'static str, usize), icon: IconName, tip: String| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .size(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.fill(Fill::Selected)))
                .tooltip(Tooltip::text(tip))
                .child(
                    Icon::new(icon)
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Secondary)),
                )
        };
        let (edit_fact, forget_fact, click_fact) = (fact.clone(), fact.clone(), fact.clone());
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .items_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(fg.fill(Fill::Hover)))
            .child(
                div()
                    .id(("mono-fact-body", ix))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(10.0))
                    .px_2()
                    .py_2()
                    .when(!struck, |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_editing_memory(click_fact.clone(), window, cx)
                            }))
                    })
                    .child(fact_icon(struck, fg))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(Text::Base.px())
                                    .line_height(px(20.0))
                                    .map(|el| {
                                        if struck {
                                            el.text_color(fg.ink(Ink::Faint)).line_through()
                                        } else {
                                            el.text_color(fg.ink(Ink::Strong))
                                        }
                                    })
                                    .child(fact.text.clone()),
                            )
                            .children(fact.date.as_deref().map(|date| {
                                div()
                                    .text_size(Text::Sm.px())
                                    .line_height(px(16.0))
                                    .text_color(fg.ink(Ink::Muted))
                                    .child(short_date(date))
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .pr_2()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .when(!struck, |el| {
                        el.child(
                            action(("mono-fact-edit", ix), IconName::Pencil, "Edit".into())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_editing_memory(edit_fact.clone(), window, cx)
                                })),
                        )
                    })
                    .child(
                        action(
                            ("mono-fact-forget", ix),
                            IconName::Trash2,
                            format!("Forget: {}", fact.text),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.forget_memory(forget_fact.clone(), cx)
                        })),
                    ),
            )
            .into_any_element()
    }

    /// MonoCode `FactEditor`: the fact typed in place (`bg-content/5`);
    /// Enter or leaving the field saves, Escape closes it as it was.
    fn render_fact_editor(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(6.0))
            .bg(fg.fill(Fill::Hover))
            .px_2()
            .py_2()
            .child(fact_icon(false, fg))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(28.0))
                    .flex()
                    .items_center()
                    .text_size(Text::Base.px())
                    .when(self.monos.files.memory_saving, |el| el.opacity(0.6))
                    .child(self.monos.files.memory_input.clone()),
            )
            .into_any_element()
    }
}

/// MonoCode `FactIcon`: `size-7 rounded-md bg-content/6`, a sticky note.
fn fact_icon(dim: bool, fg: Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .size(px(28.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(fg.fill(Fill::Hover))
        .child(
            Icon::new(IconName::StickyNote)
                .size(IconSize::Sm)
                .color(fg.ink(if dim { Ink::Faint } else { Ink::Secondary })),
        )
}

/// MonoCode `MemoryGauge`: how much of `MEMORY.md` loads against what a
/// turn has room for; amber from 80%, red once lines stop loading.
fn memory_gauge(memory: &str, cx: &Context<BenCodeApp>) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    let budget = memory_within_budget(memory);
    let share = budget.share();
    let tone = if budget.dropped_lines > 0 {
        colors.danger.ink(Ink::Body)
    } else if share >= 0.8 {
        colors.warning.ink(Ink::Body)
    } else {
        fg.ink(Ink::Faint)
    };
    div()
        .mt_2()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .text_size(Text::Sm.px())
        .line_height(px(16.0))
        .text_color(fg.ink(Ink::Muted))
        .child(
            div()
                .flex_1()
                .h(px(4.0))
                .overflow_hidden()
                .rounded_full()
                .bg(fg.fill(Fill::Selected))
                .child(
                    div()
                        .h_full()
                        .w(relative(share.clamp(0.0, 1.0)))
                        .rounded_full()
                        .bg(tone),
                ),
        )
        .child(div().flex_none().child(budget.label()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_monocode_shows_them() {
        assert_eq!(short_date("2026-10-04"), "Oct 4");
        assert_eq!(short_date("soon"), "soon");
    }
}
