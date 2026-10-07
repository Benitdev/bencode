//! MonoCode `LinkSessionWorkItemDialog`: link a thread to a GitHub issue or
//! pull request by URL (or edit / remove the link), and open the linked
//! item from the card's badge.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    MouseButton, StatefulInteractiveElement, Styled, div, relative, rgb,
};

use crate::app::BenCodeApp;
use crate::db::{LinkedWorkItem, WorkItemKind};
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;

/// MonoCode `text-red-400`.
const RED_400: u32 = 0xf87171;
/// Tailwind preflight's `line-height: 1.5` (GPUI defaults to ~1.618).
const PREFLIGHT_LEADING: f32 = 1.5;

/// The dialog's thread and its last error.
#[derive(Clone, Debug, Default)]
pub struct LinkDialog {
    pub session_id: String,
    pub error: Option<String>,
}

impl BenCodeApp {
    pub fn open_link_dialog(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let current = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.linked_work_item.as_ref())
            .map(|item| item.url.clone())
            .unwrap_or_default();
        self.link_input.update(cx, |input, cx| input.set_text(current, cx));
        self.link_dialog = Some(LinkDialog {
            session_id: session_id.to_string(),
            error: None,
        });
        let handle = gpui::Focusable::focus_handle(self.link_input.read(cx), cx);
        crate::ui::composer::focus_later(handle, cx);
        cx.notify();
    }

    fn close_link_dialog(&mut self, cx: &mut Context<Self>) {
        if self.link_dialog.take().is_some() {
            self.refocus_prompt(cx);
            cx.notify();
        }
    }

    /// MonoCode's submit: the URL must parse, else the error shows.
    pub fn submit_link_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.link_dialog.as_mut() else {
            return;
        };
        let text = self.link_input.read(cx).text().trim().to_string();
        match LinkedWorkItem::parse_url(&text) {
            Some(item) => {
                let id = dialog.session_id.clone();
                if self.set_session_link(&id, Some(item), cx) {
                    self.close_link_dialog(cx);
                }
            }
            None => {
                dialog.error = Some("Enter a valid GitHub issue or pull request URL.".into());
                cx.notify();
            }
        }
    }

    /// MonoCode `onSetHistorySessionLinkedWorkItem`: saved at once; the
    /// card follows only if the write succeeds.
    /// Returns whether it was saved; a failure stays on the dialog.
    pub fn set_session_link(
        &mut self,
        session_id: &str,
        item: Option<LinkedWorkItem>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Err(err) = self.db.set_linked_work_item(session_id, item.as_ref()) {
            log::error!("could not save the GitHub link of {session_id}: {err:#}");
            if let Some(dialog) = self.link_dialog.as_mut() {
                dialog.error = Some(err.to_string());
            }
            cx.notify();
            return false;
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.linked_work_item = item;
        }
        cx.notify();
        true
    }

    pub fn render_link_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.link_dialog.as_ref()?;
        let session = self.sessions.iter().find(|s| s.id == dialog.session_id)?;
        let linked = session.linked_work_item.is_some();
        let theme = cx.theme();
        let colors = &theme.colors;
        let title = crate::app::session_list::display_title(&session.title, &session.harness);
        let close = app_callback(cx, |this, cx| this.close_link_dialog(cx));
        let fg = colors.fg;
        let red_400: Hsla = rgb(RED_400).into();
        let invalid = dialog.error.is_some();
        // MonoCode's `label flex flex-col gap-1.5` inside a `text-[12px]` form.
        let field = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .text_size(px(12.0))
            .line_height(relative(PREFLIGHT_LEADING))
            // `font-medium text-content/80`
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.opacity(0.8))
                    .child("Issue or pull request URL"),
            )
            // `h-9 rounded-md border bg-content/5 px-2.5 text-[13px]
            // text-content`, `border-red-400/60` when invalid, else
            // `border-content/10` (`focus:border-content/30`; the field
            // holds focus while the dialog is open). Drawn here, not with
            // Ely `Input`, whose frame has other metrics.
            .child(
                div()
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(if invalid { red_400.opacity(0.6) } else { fg.opacity(0.3) })
                    .bg(fg.opacity(0.05))
                    .text_size(px(13.0))
                    .text_color(fg)
                    // Like Ely `Input`: a press on the frame focuses the field.
                    .on_mouse_down(MouseButton::Left, {
                        let handle = gpui::Focusable::focus_handle(self.link_input.read(cx), cx);
                        move |_, window, cx| window.focus(&handle, cx)
                    })
                    .child(div().flex_1().min_w_0().child(self.link_input.clone())),
            )
            // `text-[11px]`: the error in `text-red-400`, else the hint in
            // `text-content/45`.
            .child(match dialog.error.clone() {
                Some(error) => div().text_size(px(11.0)).text_color(red_400).child(error),
                None => div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .child("Paste the full github.com URL. The linked item will appear on the session card."),
            });
        let remove = app_callback(cx, |this, cx| {
            if let Some(id) = this.link_dialog.as_ref().map(|d| d.session_id.clone())
                && this.set_session_link(&id, None, cx)
            {
                this.close_link_dialog(cx);
            }
        });
        let submit = app_callback(cx, |this, cx| this.submit_link_dialog(cx));
        let mut built = Dialog::new(
            "link-work-item",
            if linked {
                "Edit GitHub link"
            } else {
                "Link GitHub issue or PR"
            },
            close,
        )
        .detail(title)
        .child(field);
        if linked {
            // MonoCode `mr-auto rounded-md px-3 py-1.5 text-red-400
            // hover:bg-red-400/10`: red text, not Ely's filled Danger.
            built = built.action(move |_| {
                div()
                    .id("link-remove")
                    .mr_auto()
                    .px_3()
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .line_height(relative(PREFLIGHT_LEADING))
                    .text_color(red_400)
                    .hover(move |s| s.bg(red_400.opacity(0.1)))
                    .on_click(move |_, window, cx| remove(window, cx))
                    .child("Remove link")
            });
        }
        let dialog = built
            .action(move |close| {
                Button::new("link-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new("link-submit", if linked { "Update link" } else { "Link" })
                    .primary()
                    .on_click(move |_, window, cx| submit(window, cx))
            });
        Some(dialog.into_any_element())
    }

    /// The card badge's click: the item in the Inbox, fetched first when
    /// the Inbox has not listed it.
    pub fn open_linked_work_item(&mut self, session_id: &str, item: &LinkedWorkItem, cx: &mut Context<Self>) {
        let kind = match item.kind {
            WorkItemKind::Pr => crate::github::Kind::Pr,
            WorkItemKind::Issue => crate::github::Kind::Issue,
        };
        let key = format!(
            "{}:{}:{}",
            item.repo.to_lowercase(),
            if kind == crate::github::Kind::Pr { "pr" } else { "issue" },
            item.number
        );
        self.open_inbox_modal(cx);
        if self.inbox.item(&key).is_some() {
            self.select_inbox_key(key, cx);
            return;
        }
        let cwd = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.work_dir().to_string())
            .unwrap_or_else(|| self.current_cwd.clone());
        let (repo, number) = (item.repo.clone(), item.number);
        let task = cx.background_executor().spawn(async move {
            crate::github::work_item(std::path::Path::new(&cwd), &repo, kind, number).map(|mut found| {
                found.project = cwd;
                found
            })
        });
        let url = item.url.clone();
        cx.spawn(async move |this, cx| {
            let fetched = task.await;
            let updated = this.update(cx, |this, cx| match fetched {
                Ok(found) => {
                    let key = found.key();
                    if this.inbox.item(&key).is_none() {
                        this.inbox.items.push(found);
                    }
                    this.select_inbox_key(key, cx);
                }
                Err(err) => {
                    // Without `gh`, the browser still shows it.
                    log::warn!("could not load {url}: {err}");
                    cx.open_url(&url);
                }
            });
            if let Err(err) = updated {
                log::debug!("linked item loaded after app drop: {err:#}");
            }
        })
        .detach();
    }
}
