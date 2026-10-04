//! MonoCode `McpServerPicker`: `/mcp` opens a searchable list of the
//! configured MCP servers above the composer. Picking one puts its
//! `@mcp/name` tag in the prompt; a turn that still holds the tag is sent
//! with an "MCP context" line naming the server.

use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use super::focus_later;
use super::mcp_tags::{self, Availability, PickerServer};
use super::search_popover::{SearchPopover, footer_action};
use crate::app::BenCodeApp;
use crate::harness::catalog;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::settings_modal::SettingsTab;

/// MonoCode `max-h-[min(184px,45vh)]` and its 44px rows.
const LIST_MAX_HEIGHT: gpui::Pixels = px(184.0);
const ROW_HEIGHT: gpui::Pixels = px(44.0);

/// The open picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct McpPicker {
    /// The highlighted row (always an available one when any is).
    pub active: usize,
    /// Where the tag goes: where `/mcp` stood, else the caret.
    pub insert_at: Option<usize>,
}

/// The next available row from `active`, `delta` steps away, wrapping.
fn step_available(rows: &[PickerServer], active: usize, delta: isize) -> usize {
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.availability == Availability::Available)
        .map(|(ix, _)| ix)
        .collect();
    if selectable.is_empty() {
        return active;
    }
    let at = selectable.iter().position(|&ix| ix == active).unwrap_or(0) as isize;
    let len = selectable.len() as isize;
    selectable[(at + delta).rem_euclid(len) as usize]
}

fn first_available(rows: &[PickerServer]) -> usize {
    rows.iter()
        .position(|row| row.availability == Availability::Available)
        .unwrap_or(0)
}

/// `text` with `token` inserted at `at` as its own word, and the caret
/// after it (MonoCode's leading and trailing spaces).
fn insert_token(text: &str, at: usize, token: &str) -> (String, usize) {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let (before, after) = text.split_at(at);
    let leading = if before.is_empty() || before.ends_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let trailing = if after.starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let insertion = format!("{leading}{token}{trailing}");
    (format!("{before}{insertion}{after}"), at + insertion.len())
}

impl BenCodeApp {
    /// The harness the focused composer sends to (`claude`, `codex`, …).
    fn composer_harness(&self) -> &'static str {
        catalog::find(&self.current_model_key()).map_or("claude", |m| m.harness.id())
    }

    fn mcp_rows(&self, cx: &Context<Self>) -> Vec<PickerServer> {
        let Some(servers) = &self.integrations.mcp_servers else {
            return Vec::new();
        };
        mcp_tags::picker_servers(
            servers,
            self.composer_harness(),
            &self.integrations.mcp_health,
            self.mcp_search_input.read(cx).text(),
        )
    }

    /// Opens the picker; the tag will go at `insert_at` (else the caret).
    pub fn open_mcp_picker(&mut self, insert_at: Option<usize>, cx: &mut Context<Self>) {
        self.is_skill_picker_open = false;
        self.is_mention_picker_open = false;
        self.mcp_picker = Some(McpPicker {
            active: 0,
            insert_at,
        });
        self.mcp_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        let claude = self.composer_harness() == "claude";
        self.refresh_mcp_servers(claude, cx);
        self.on_mcp_query_changed(cx);
        focus_later(self.mcp_search_input.read(cx).focus_handle(cx), cx);
    }

    /// Closes the picker; Esc and the chevron hand focus back to the prompt.
    /// True when it was open.
    pub fn close_mcp_picker(&mut self, refocus: bool, cx: &mut Context<Self>) -> bool {
        if self.mcp_picker.take().is_none() {
            return false;
        }
        if refocus {
            self.refocus_prompt(cx);
        }
        cx.notify();
        true
    }

    /// The search changed (or the list arrived): highlight the first usable row.
    pub fn on_mcp_query_changed(&mut self, cx: &mut Context<Self>) {
        let active = first_available(&self.mcp_rows(cx));
        if let Some(picker) = &mut self.mcp_picker {
            picker.active = active;
        }
        cx.notify();
    }

    /// `/mcp` from the `/` list: the token leaves the prompt and the picker
    /// opens where it stood.
    pub fn start_mcp_command(&mut self, cx: &mut Context<Self>) {
        let at = self.remove_prompt_token(cx);
        self.open_mcp_picker(Some(at), cx);
    }

    /// ↑/↓, Enter and Esc in the picker's search.
    pub fn mcp_picker_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.mcp_picker else {
            return false;
        };
        match key {
            "up" | "down" => {
                let rows = self.mcp_rows(cx);
                let delta = if key == "up" { -1 } else { 1 };
                let active = step_available(&rows, picker.active, delta);
                if let Some(picker) = &mut self.mcp_picker {
                    picker.active = active;
                }
                self.picker_scroll.scroll_to_item(active);
                cx.notify();
                true
            }
            "enter" => {
                self.pick_mcp_server(picker.active, cx);
                true
            }
            "escape" => self.close_mcp_picker(true, cx),
            _ => false,
        }
    }

    /// MonoCode `onPick`: reuse the server's tag, put it in the prompt
    /// unless it is already there, and go back to the prompt.
    fn pick_mcp_server(&mut self, ix: usize, cx: &mut Context<Self>) {
        let rows = self.mcp_rows(cx);
        let Some(row) = rows
            .get(ix)
            .filter(|row| row.availability == Availability::Available)
        else {
            return;
        };
        let Some(picker) = self.mcp_picker.take() else {
            return;
        };
        let text = self.prompt_input.read(cx).text().to_string();
        let tags = self.mcp_tags.borrow().clone();
        let tag = match mcp_tags::tag_for(&tags, &row.server) {
            Some(tag) => tag.clone(),
            None => {
                let tag = mcp_tags::new_mcp_tag(&row.server, &tags);
                let mut next = (*tags).clone();
                next.push(tag.clone());
                *self.mcp_tags.borrow_mut() = next.into();
                tag
            }
        };
        if mcp_tags::tagged_servers(&text, std::slice::from_ref(&tag)).is_empty() {
            let at = picker
                .insert_at
                .unwrap_or_else(|| self.prompt_input.read(cx).cursor());
            let (next, caret) = insert_token(&text, at, &tag.token);
            self.prompt_input.update(cx, |input, cx| {
                input.set_text(next, cx);
                input.select(caret..caret, cx);
            });
        }
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// The turn as sent: the "MCP context" line for the tags still in
    /// `text`. The tags are spent either way.
    pub fn take_mcp_context(&mut self, text: String) -> String {
        let tags = std::mem::take(&mut *self.mcp_tags.borrow_mut());
        let servers = mcp_tags::tagged_servers(&text, &tags);
        mcp_tags::mcp_context_text(&servers, &text)
    }

    fn manage_mcp_servers(&mut self, cx: &mut Context<Self>) {
        self.close_mcp_picker(false, cx);
        self.settings_tab = SettingsTab::Mcp;
        self.open_settings(cx);
    }

    pub(super) fn render_mcp_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.mcp_picker?;
        let rows = self.mcp_rows(cx);
        let empty = if self.integrations.mcp_servers.is_none() {
            Some("Checking MCP servers…")
        } else if rows.is_empty() {
            Some(if self.mcp_search_input.read(cx).text().is_empty() {
                "No MCP servers found"
            } else {
                "No matching MCP servers"
            })
        } else {
            None
        };
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                self.render_mcp_row(ix, row, picker.active == ix, cx)
                    .into_any_element()
            })
            .collect();
        Some(
            SearchPopover {
                scroll: &self.picker_scroll,
                id: "mcp-picker",
                icon: IconName::Search,
                input: &self.mcp_search_input,
                close: (
                    IconName::ChevronDown,
                    "Back to the conversation (Esc)",
                    |this, cx| {
                        this.close_mcp_picker(true, cx);
                    },
                ),
                dismiss: |this, cx| {
                    this.close_mcp_picker(false, cx);
                },
                list_max_height: LIST_MAX_HEIGHT,
                empty: empty.map(SharedString::from),
                rows,
                footer: Some(footer_action(
                    "mcp-picker-manage",
                    IconName::Settings,
                    "Manage MCP Servers…",
                    |this, cx| this.manage_mcp_servers(cx),
                    cx,
                )),
            }
            .render(cx),
        )
    }

    fn render_mcp_row(
        &self,
        ix: usize,
        row: PickerServer,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let available = row.availability == Availability::Available;
        let status_color = match row.availability {
            Availability::Authentication => colors.warning,
            _ => fg.opacity(0.45),
        };
        // MonoCode draws Claude Desktop servers with the Claude mark.
        let icon = match row.server.provider.as_str() {
            "claude_desktop" => "claude",
            provider => provider,
        };
        let subtitle = format!(
            "{} · {}",
            mcp_tags::provider_label(&row.server.provider),
            row.server.scope
        );
        div()
            .id(("mcp-server", ix))
            .flex()
            .items_center()
            .gap_2()
            .h(ROW_HEIGHT)
            .px_2()
            .rounded(px(6.0))
            .text_color(if available { fg } else { fg.opacity(0.4) })
            .when(available && active, |el| el.bg(super::selection(cx)))
            .when(available && !active, |el| {
                el.cursor_pointer().hover(move |s| s.bg(fg.opacity(0.05)))
            })
            .when(available, |el| {
                el.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if let Some(picker) = &mut this.mcp_picker
                        && *hovered
                        && picker.active != ix
                    {
                        picker.active = ix;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.pick_mcp_server(ix, cx)))
            })
            .child(
                div()
                    .flex_none()
                    .child(HarnessIcon::new(icon).size(px(16.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .child(SharedString::from(row.server.name.clone())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .child(subtitle),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(gpui::relative(0.4))
                    .truncate()
                    .text_size(px(11.0))
                    .text_color(status_color)
                    .child(row.status()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpConnection;

    fn row(name: &str, availability: Availability) -> PickerServer {
        PickerServer {
            server: McpConnection {
                provider: "claude".into(),
                name: name.into(),
                scope: "user".into(),
                config_path: String::new(),
                transport: "stdio".into(),
                enabled: true,
            },
            availability,
            detail: "",
        }
    }

    #[test]
    fn arrows_skip_rows_that_cannot_be_picked() {
        let rows = vec![
            row("a", Availability::Available),
            row("b", Availability::Authentication),
            row("c", Availability::Available),
            row("d", Availability::Unavailable),
        ];
        assert_eq!(first_available(&rows), 0);
        assert_eq!(step_available(&rows, 0, 1), 2);
        assert_eq!(step_available(&rows, 2, 1), 0);
        assert_eq!(step_available(&rows, 0, -1), 2);
        assert_eq!(step_available(&rows[3..], 0, 1), 0);
    }

    #[test]
    fn tokens_go_in_as_their_own_word() {
        assert_eq!(insert_token("", 0, "@mcp/a"), ("@mcp/a ".into(), 7));
        assert_eq!(
            insert_token("ask now", 3, "@mcp/a"),
            ("ask @mcp/a now".into(), 10)
        );
        assert_eq!(
            insert_token("ask", 99, "@mcp/a"),
            ("ask @mcp/a ".into(), 11)
        );
    }
}
