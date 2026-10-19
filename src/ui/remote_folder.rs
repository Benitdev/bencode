//! MonoCode `AddRemoteProjectDialog`: "Open folder on a machine". Pick the
//! machine, browse to a folder there (or type its path), Open: it becomes a
//! project of the rail whose threads run on that machine. State and logic:
//! `app/remote.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::{Choice, Input, Select};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, relative, rgb,
};

use crate::app::BenCodeApp;
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::scale::px;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::tokens::RED_400;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    pub fn render_remote_folder_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.remote.folder.as_ref()?;
        let fg = cx.theme().colors.fg;
        let close = app_callback(cx, |this, cx| this.close_remote_folder_dialog(cx));
        let built = Dialog::new("remote-folder", "Open folder on a machine", close).detail(
            "Threads in this project run on that machine, using its checkout and its agents' \
             sign-ins. They keep running when you close BenCode here.",
        );
        let machine = dialog
            .machine
            .as_ref()
            .and_then(|id| self.machines().iter().find(|m| &m.id == id))
            .cloned();
        let Some(machine) = machine else {
            let add = app_callback(cx, |this, cx| {
                this.close_remote_folder_dialog(cx);
                this.open_settings(cx);
                this.select_settings_tab(SettingsTab::Connections, cx);
            });
            return Some(
                built
                    .child(
                        div()
                            .text_size(Text::Base.px())
                            .line_height(relative(1.375))
                            .text_color(fg.ink(Ink::Secondary))
                            .child(
                                "No machines are connected yet. Add one in Settings, then open a \
                                 folder on it here.",
                            ),
                    )
                    .action(|close| {
                        Button::new("remote-folder-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, window, cx| close(window, cx))
                    })
                    .action(move |_| {
                        Button::new("remote-folder-add-machine", "Add a machine")
                            .primary()
                            .on_click(move |_, window, cx| add(window, cx))
                    })
                    .into_any_element(),
            );
        };

        let picker: AnyElement = if self.machines().len() > 1 {
            let machines = self
                .machines()
                .iter()
                .map(|m| Choice::new(m.id.clone(), m.name.clone()));
            Select::new("remote-folder-machine", machines)
                .label("Machine")
                .selected(machine.id.clone())
                .on_change(on_value(cx, |this, id, cx| {
                    this.pick_remote_folder_machine(id, cx)
                }))
                .into_any_element()
        } else {
            div()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Secondary))
                .child("On ")
                .child(
                    div()
                        .text_color(fg.ink(Ink::Strong))
                        .child(machine.name.clone()),
                )
                .flex()
                .into_any_element()
        };

        let row = |id: SharedString, name: SharedString, path: String| {
            let machine_id = machine.id.clone();
            let open = app_callback(cx, move |this, cx| {
                this.browse_remote_folder(&machine_id, Some(path.clone()), cx)
            });
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(Text::Base.px())
                .text_color(fg.opacity(0.75))
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                .on_click(move |_, window, cx| open(window, cx))
                .child(
                    Icon::new(IconName::Folder)
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Muted)),
                )
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Faint)),
                )
        };
        let quiet = |text: &'static str| {
            div()
                .px_2()
                .py(px(6.0))
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Muted))
                .child(text)
        };
        let mut folders = div().p_1();
        if let Some(listing) = &dialog.listing {
            if let Some(parent) = &listing.parent {
                folders =
                    folders.child(row("remote-folder-up".into(), "..".into(), parent.clone()));
            }
            for (ix, name) in listing.folders.iter().enumerate() {
                let path = format!("{}/{name}", listing.path.trim_end_matches('/'));
                folders = folders.child(row(
                    SharedString::from(format!("remote-folder-{ix}")),
                    name.clone().into(),
                    path,
                ));
            }
            if listing.folders.is_empty() {
                folders = folders.child(quiet("No subfolders"));
            }
        } else if dialog.loading {
            folders = folders.child(quiet("Loading folders…"));
        }
        let list = div()
            .id("remote-folder-list")
            .min_h(px(96.0))
            .max_h(px(320.0))
            .overflow_y_scroll()
            .rounded(px(6.0))
            .border_1()
            .border_color(fg.fill(Fill::Strong))
            .child(folders);
        let red_400: Hsla = rgb(RED_400).into();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(picker)
            .child(Input::new(&self.remote.path_input))
            .child(list)
            .children(dialog.error.clone().map(|error| {
                div()
                    .text_size(Text::Base.px())
                    .line_height(relative(1.375))
                    .text_color(red_400.opacity(0.9))
                    .child(error)
            }));
        let typed = !self.remote.path_input.read(cx).text().trim().is_empty();
        let opening = dialog.opening;
        let open = app_callback(cx, |this, cx| this.open_remote_folder(cx));
        Some(
            built
                .child(body)
                .action(|close| {
                    Button::new("remote-folder-cancel", "Cancel")
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .action(move |_| {
                    Button::new(
                        "remote-folder-open",
                        if opening { "Opening…" } else { "Open" },
                    )
                    .primary()
                    .disabled(opening || !typed)
                    .on_click(move |_, window, cx| open(window, cx))
                })
                .into_any_element(),
        )
    }
}
