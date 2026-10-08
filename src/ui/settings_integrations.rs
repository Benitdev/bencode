//! Settings › Integrations. GitHub: the accounts `gh` is signed in with
//! and the one each project runs as (BenCode's own; state and logic in
//! `app/github_accounts.rs`). Nulab Backlog (shaped after MonoCode's
//! `JiraSettings.tsx`): connect with the space's address and a personal
//! API key, pick which projects the Inbox lists, disconnect behind one
//! confirmation. State and logic: `app/backlog.rs`.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::{Input, PasswordInput, Switch};
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::settings::{SettingsRow, SettingsSection};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, relative};

use crate::app::BenCodeApp;
use crate::backlog::Account;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::scale::px;

/// The connection form's fields.
const FIELD_WIDTH: f32 = 280.0;

impl BenCodeApp {
    pub(crate) fn render_settings_integrations(&self, cx: &Context<Self>) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.render_github_accounts())
            .children(self.render_github_projects(cx));
        match &self.backlog.account {
            Some(account) => page
                .child(self.render_backlog_connection(account, cx))
                .child(self.render_backlog_projects(cx)),
            None => page.child(self.render_backlog_form(cx)),
        }
    }

    /// The accounts `gh` is signed in with.
    fn render_github_accounts(&self) -> SettingsSection {
        let section = SettingsSection::new("GitHub").description(
            "The accounts the GitHub CLI is signed in with. Add one with `gh auth login` in a \
             terminal; BenCode keeps no token of its own.",
        );
        match &self.github.accounts {
            None => section.row(SettingsRow::new("Reading accounts…")),
            Some(Err(err)) => section.row(SettingsRow::new("Could not read gh's accounts").description(err.clone())),
            Some(Ok(accounts)) if accounts.is_empty() => section.row(
                SettingsRow::new("Not signed in").description("Run `gh auth login` in a terminal to connect GitHub."),
            ),
            Some(Ok(accounts)) => accounts.iter().fold(section, |section, account| {
                let row = SettingsRow::new(account.login.clone());
                section.row(if account.active {
                    row.description("Active in gh: what a project on Automatic uses first.")
                } else {
                    row
                })
            }),
        }
    }

    /// Each rail project's account, once there are two to pick from.
    fn render_github_projects(&self, cx: &Context<Self>) -> Option<SettingsSection> {
        let accounts = self.github.accounts.as_ref()?.as_ref().ok().filter(|accounts| accounts.len() > 1)?;
        let section = SettingsSection::new("GitHub account by project").description(
            "The account a project's Inbox, pull requests, comments and merges run as. Automatic \
             uses the active account, or another one when the active one cannot see the repository.",
        );
        Some(self.rail_order().into_iter().enumerate().fold(section, |section, (ix, project)| {
            let chosen = self.github.choices.get(&project).cloned();
            let pick = |login: Option<String>| {
                let project = project.clone();
                app_callback(cx, move |this, cx| this.set_project_github_account(&project, login.clone(), cx))
            };
            let menu = accounts.iter().fold(
                Menu::new().item(MenuItem::radio("Automatic", chosen.is_none()).on_click(pick(None))),
                |menu, account| {
                    let on = chosen.as_deref() == Some(account.login.as_str());
                    menu.item(MenuItem::radio(account.login.clone(), on).on_click(pick(Some(account.login.clone()))))
                },
            );
            let label = chosen.unwrap_or_else(|| "Automatic".to_string());
            section.row(
                SettingsRow::new(self.rail_project_label(&project))
                    .description(project.clone())
                    .control(DropdownMenu::new(("github-project-account", ix), label, menu).variant(ButtonVariant::Outline)),
            )
        }))
    }

    fn backlog_section(&self) -> SettingsSection {
        SettingsSection::new("Backlog")
            .description("Open issues of a Nulab Backlog space show in the Inbox, beside GitHub's.")
    }

    /// Not connected: the space, the key, Connect.
    fn render_backlog_form(&self, cx: &Context<Self>) -> SettingsSection {
        let busy = self.backlog.busy || !self.backlog.loaded;
        let typed = !self.backlog_space_input.read(cx).text().trim().is_empty()
            && !self.backlog_key_input.read(cx).text().trim().is_empty();
        let field = |input: AnyElement| div().w(px(FIELD_WIDTH)).child(input);
        let error = self.backlog.error.clone();
        self.backlog_section()
            .row(
                SettingsRow::new("Space")
                    .description("Your space's address, like yourspace.backlog.com or yourspace.backlog.jp")
                    .control(field(Input::new(&self.backlog_space_input).into_any_element())),
            )
            .row(
                SettingsRow::new("API key")
                    .description(
                        "From Backlog › Personal settings › API. Saved on this Mac, in a file only \
                         your account can read.",
                    )
                    .control(field(PasswordInput::new(&self.backlog_key_input).into_any_element())),
            )
            .row(
                SettingsRow::new(if self.backlog.busy { "Checking the key…" } else { "Connect" })
                    .description(error.unwrap_or_else(|| {
                        "BenCode reads issues and comments, and posts the comments and status \
                         changes you make in the Inbox."
                            .to_string()
                    }))
                    .control(
                        Button::new("backlog-connect", "Connect")
                            .primary()
                            .disabled(busy || !typed)
                            .on_click(cx.listener(|this, _, _, cx| this.connect_backlog(cx))),
                    ),
            )
    }

    /// Connected: who and where, and Disconnect.
    fn render_backlog_connection(&self, account: &Account, cx: &Context<Self>) -> SettingsSection {
        let who = if account.user_name.is_empty() {
            account.space.clone()
        } else {
            format!("{} · {}", account.user_name, account.space)
        };
        let detail = match &self.backlog.error {
            Some(err) => format!("{who}\n{err}"),
            None => who,
        };
        self.backlog_section().row(
            SettingsRow::new("Connected").description(detail).control(
                Button::new("backlog-disconnect", "Disconnect")
                    .variant(ButtonVariant::Outline)
                    .disabled(self.backlog.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.backlog_disconnect_open = true;
                        cx.notify();
                    })),
            ),
        )
    }

    /// The space's projects, each with a switch for the Inbox.
    fn render_backlog_projects(&self, cx: &Context<Self>) -> SettingsSection {
        let section = SettingsSection::new("Backlog projects")
            .description("Projects whose open issues the Inbox lists.");
        let projects = match &self.backlog.projects {
            None => return section.row(SettingsRow::new("Loading projects…")),
            Some(Err(err)) => {
                return section.row(SettingsRow::new("Could not list projects").description(err.clone()));
            }
            Some(Ok(projects)) if projects.is_empty() => {
                return section.row(SettingsRow::new("No projects in this space"));
            }
            Some(Ok(projects)) => projects,
        };
        projects.iter().fold(section, |section, project| {
            let shown = !self.backlog.hidden_projects.contains(&project.id);
            let id = project.id.clone();
            section.row(
                SettingsRow::new(project.name.clone())
                    .description(project.key.clone())
                    .control(
                        Switch::new(SharedString::from(format!("backlog-project-{}", project.id)), shown)
                            .on_change(app_callback_with(cx, move |this, on, cx| {
                                this.set_backlog_project_shown(&id, on, cx)
                            })),
                    ),
            )
        })
    }

    /// Disconnect's confirmation: the key is deleted.
    pub fn render_backlog_disconnect(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.backlog_disconnect_open {
            return None;
        }
        let space = self.backlog.account.as_ref().map(|a| a.space.clone()).unwrap_or_default();
        let close = app_callback(cx, |this, cx| {
            this.backlog_disconnect_open = false;
            cx.notify();
        });
        let confirm = app_callback(cx, |this, cx| {
            this.backlog_disconnect_open = false;
            this.disconnect_backlog(cx);
        });
        let dialog = Dialog::new("backlog-disconnect-dialog", "Disconnect Backlog", close)
            .child(
                div()
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .text_color(cx.theme().colors.fg_muted)
                    .child(format!(
                        "Disconnect {space}? The saved API key is deleted and its issues leave the \
                         Inbox. Threads started from them stay."
                    )),
            )
            .action(|close| {
                Button::new("backlog-disconnect-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new("backlog-disconnect-confirm", "Disconnect")
                    .variant(ButtonVariant::Danger)
                    .on_click(move |_, window, cx| confirm(window, cx))
            });
        Some(dialog.into_any_element())
    }
}
