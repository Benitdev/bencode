//! Settings › Connections (MonoCode `ConnectionsSettings.tsx`): the
//! machines agents run on, each with its connection, Reconnect, Update
//! Host and Remove; Add machine's SSH form; and a setup's progress, where
//! SSH's questions are answered. State and logic: `app/remote.rs`.
//!
//! BenCode connects through SSH alone, so MonoCode's "Connect to an
//! existing host by URL" and its device tokens (Revoke access) are not here.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::motion::Spinner;
use ely_gpui_component::forms::{Input, PasswordInput};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
    relative,
};

use crate::app::BenCodeApp;
use crate::remote::Status;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::settings_parts::{SettingsGroup, SettingsPage, SettingsRow, icon_tile};

/// The form's fields.
const FIELD_WIDTH: f32 = 300.0;

impl BenCodeApp {
    pub(crate) fn render_settings_connections(&self, cx: &Context<Self>) -> SettingsPage {
        let mut page = SettingsTab::Connections
            .page()
            .group(self.render_machines(cx));
        if self.remote.adding {
            page = page.group(self.render_add_machine(cx));
        }
        if let Some(progress) = self.render_machine_setup(cx) {
            page = page.group(progress);
        }
        page
    }

    /// MonoCode's "Your machines" list.
    fn render_machines(&self, cx: &Context<Self>) -> SettingsGroup {
        let fg = cx.theme().colors.fg;
        let busy = self.remote.job.is_some();
        let mut group = SettingsGroup::new("Your machines").description(
            "Run agents on another computer and return to them from your laptop. The host keeps \
             working when you close BenCode here.",
        );
        if !self.remote.adding {
            group = group.action(
                Button::new("connections-add", "Add machine")
                    .icon(IconName::Plus)
                    .size(ControlSize::Sm)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.open_add_machine(cx))),
            );
        }
        if self.machines().is_empty() {
            return if self.remote.adding {
                group.note("Connect through SSH below.")
            } else {
                group.note("Add your always-on Mac or Linux machine to get started.")
            };
        }
        let rows = self.machines().iter().map(|machine| {
            let status = self.machine_status(&machine.id);
            let (line, update) = match &status {
                Status::Connected(info) if info.version != env!("CARGO_PKG_VERSION") => (
                    format!(
                        "Connected · host {} (this app is {})",
                        info.version,
                        env!("CARGO_PKG_VERSION")
                    ),
                    true,
                ),
                Status::Connected(info) if info.running > 0 => (
                    format!(
                        "Connected · {} running on {}",
                        if info.running == 1 {
                            "1 agent".to_string()
                        } else {
                            format!("{} agents", info.running)
                        },
                        info.name
                    ),
                    false,
                ),
                Status::Connected(_) => ("Connected".to_string(), false),
                Status::Connecting | Status::Idle => ("Checking connection…".to_string(), false),
                Status::Offline(err) => (format!("Offline · {err}"), false),
            };
            let description = if update {
                format!(
                    "{}\n{line}\nUpdating restarts the host and interrupts active agent turns.",
                    machine.address()
                )
            } else {
                format!("{}\n{line}", machine.address())
            };
            let row = SettingsRow::new(machine.name.clone()).leading(icon_tile(
                Icon::new(IconName::Globe)
                    .size(IconSize::Sm)
                    .color(fg.opacity(0.45)),
                fg,
            ));
            let row = if matches!(status, Status::Offline(_)) {
                row.error(description)
            } else {
                row.description(description)
            };
            let id = machine.id.clone();
            let update_id = machine.id.clone();
            let remove_id = machine.id.clone();
            row.control(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(update, |controls| {
                        controls.child(
                            Button::new(
                                SharedString::from(format!("connections-update-{}", machine.id)),
                                "Update Host",
                            )
                            .variant(ButtonVariant::Outline)
                            .size(ControlSize::Sm)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_machine_host(&update_id, cx)
                            })),
                        )
                    })
                    .child(
                        Button::new(
                            SharedString::from(format!("connections-reconnect-{}", machine.id)),
                            "Reconnect",
                        )
                        .variant(ButtonVariant::Outline)
                        .size(ControlSize::Sm)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.reconnect_machine(&id, cx))),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("connections-remove-{}", machine.id)),
                            IconName::Trash2,
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .tooltip("Remove connection…")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_remove_machine(&remove_id, cx)
                        })),
                    ),
            )
        });
        let mut group = group.rows(rows.collect::<Vec<_>>());
        if let Some(error) = self.remote.error.clone().filter(|_| !self.remote.adding) {
            group = group.row(SettingsRow::new("Could not connect").error(error));
        }
        if let Some(notice) = self.remote.notice.clone() {
            group = group.row(SettingsRow::new("Done").description(notice));
        }
        group
    }

    /// MonoCode's "Connect through SSH" form.
    fn render_add_machine(&self, cx: &Context<Self>) -> SettingsGroup {
        let busy = self.remote.job.is_some();
        let typed = !self.remote.target_input.read(cx).text().trim().is_empty();
        let field = |input: AnyElement| div().w(px(FIELD_WIDTH)).child(input);
        let mut group = SettingsGroup::new("Connect through SSH")
            .description(
                "BenCode installs and starts its background host, then connects securely. Your \
                 SSH keys and config are used automatically. Enable SSH on the machine and sign in \
                 to Claude Code, Codex or your other agents there. On a Mac, keep its account \
                 signed in and the machine awake; locking the screen is fine. On Linux, setup \
                 turns on lingering for your account (`loginctl enable-linger`), so the host keeps \
                 running after you log out. The host runs until you stop it on that machine; \
                 removing it here only disconnects this computer.",
            )
            .row(
                SettingsRow::new("SSH address")
                    .description("An SSH host or an alias from your SSH config.")
                    .control(field(Input::new(&self.remote.target_input).into_any_element())),
            )
            .row(
                SettingsRow::new("Name")
                    .description("Optional. The machine's own name otherwise.")
                    .control(field(Input::new(&self.remote.name_input).into_any_element())),
            )
            .row(
                SettingsRow::new("SSH port")
                    .description("Advanced. Leave empty to use your SSH config.")
                    .control(div().w(px(160.0)).child(Input::new(&self.remote.port_input))),
            );
        if let Some(error) = self.remote.error.clone() {
            group = group.row(SettingsRow::new("Could not connect").error(error));
        }
        group.row(
            SettingsRow::new("").control(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("connections-add-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_add_machine(cx))),
                    )
                    .child(
                        Button::new("connections-add-connect", if busy { "Connecting…" } else { "Connect" })
                            .primary()
                            .size(ControlSize::Sm)
                            .loading(busy)
                            .disabled(busy || !typed)
                            .on_click(cx.listener(|this, _, _, cx| this.add_machine(cx))),
                    ),
            ),
        )
    }

    /// A setup's progress and SSH's question (MonoCode's `role="status"` box).
    fn render_machine_setup(&self, cx: &Context<Self>) -> Option<SettingsGroup> {
        let (job, _) = self.remote.job.as_ref()?;
        let view = job.view();
        let fg = cx.theme().colors.fg;
        let status = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.0))
            .child(Spinner::new("connections-setup-spinner"))
            .child(view.message.clone());
        let mut group = SettingsGroup::new("Connecting").row(SettingsRow::new("").control(status));
        if let Some(prompt) = view.prompt {
            let message = div()
                .text_size(px(12.0))
                .line_height(relative(1.625))
                .text_color(fg.opacity(0.7))
                .child(prompt.message.clone());
            let answer: AnyElement = if prompt.confirm {
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("connections-trust", "Trust host and continue")
                            .primary()
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.answer_ssh_prompt(Some("yes"), cx))),
                    )
                    .child(
                        Button::new("connections-reject", "Reject")
                            .variant(ButtonVariant::Outline)
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.answer_ssh_prompt(Some("no"), cx))),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(FIELD_WIDTH)).child(PasswordInput::new(&self.remote.answer_input)))
                    .child(
                        Button::new("connections-answer", "Continue")
                            .primary()
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| this.answer_ssh_prompt(None, cx))),
                    )
                    .into_any_element()
            };
            group = group.row(
                div()
                    .px_4()
                    .py(px(14.0))
                    .child(message)
                    .child(div().mt(px(12.0)).child(answer)),
            );
        }
        Some(
            group.row(
                SettingsRow::new("").control(
                    Button::new("connections-cancel", "Cancel connection")
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_machine_setup(cx))),
                ),
            ),
        )
    }

    /// Remove's confirmation (MonoCode's inline confirm, with one choice:
    /// BenCode holds no device credential to revoke).
    pub fn render_machine_removal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.remote.removing.clone()?;
        let machine = self.machines().iter().find(|m| m.id == id)?.clone();
        let close = app_callback(cx, |this, cx| {
            this.remote.removing = None;
            cx.notify();
        });
        let remove = app_callback(cx, move |this, cx| this.remove_machine(&id, cx));
        let body = format!(
            "This closes BenCode's connection to {name}. It does not stop the host: agents running \
             there keep running, and threads in its projects stay here. You can add it again later.\n\n\
             To stop the host, run `~/.bencode-host/bin/bencode-host stop` on that machine.",
            name = machine.name
        );
        Some(
            ely_gpui_component::overlays::ConfirmDialog::new(
                "remove-machine",
                format!("Remove {} from this computer?", machine.name),
                body,
                close,
            )
            .confirm("Remove")
            .destructive()
            .on_confirm(remove)
            .into_any_element(),
        )
    }
}
