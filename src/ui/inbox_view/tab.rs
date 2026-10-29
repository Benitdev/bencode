//! BenCode's own: an Inbox item as a tab beside the chat
//! (`PaneTab::WorkItem`), so the ticket stays readable while its thread is
//! steered. The tab draws the Inbox's detail for its item
//! (`DetailView::Tab`): the head, properties, description and
//! conversation; commenting stays in the Inbox, one click away.
//!
//! It opens from the detail ("Open beside the chat"), from a thread's work
//! item badge, and with "Send to agent" / "Open thread". An item the Inbox
//! has not got (never opened, or filtered out) is fetched off the UI
//! thread by its key into `InboxState::known`.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::feedback::EmptyState;
use bencode_ui::primitives::IconName;
use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use super::{DetailView, loading_icon, short_key};
use crate::app::file_pane::PaneTab;
use crate::app::{BenCodeApp, Surface};
use crate::db::{LinkedWorkItem, WorkItemKind};
use crate::github::{self, Kind};
use crate::ui::tokens::{Ink, Text, Tone as _};
use crate::work_items::Provider;

impl BenCodeApp {
    /// Opens Inbox item `key` in a tab beside the thread in view, out of
    /// the surface or Mono over it. An item the Inbox has not got is
    /// fetched, through `link` when given, else by its key. `pin` keeps
    /// the tab; else it is a preview the next one replaces.
    pub(crate) fn open_work_item_tab(
        &mut self,
        key: &str,
        link: Option<&LinkedWorkItem>,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        self.surface_shown_session(None, cx);
        if self.leave_bot_view(cx) {
            self.sync_selection(cx);
        }
        if self.surface.is_some_and(|s| s != Surface::Settings) {
            self.close_surface(cx);
        }
        // A fetch that failed is tried again.
        self.inbox_mut(cx, |inbox| {
            if matches!(inbox.known.get(key), Some(Err(_))) {
                inbox.known.value.remove(key);
            }
        });
        if let Some(link) = link
            && self.inbox(cx).item(key).is_none()
        {
            self.fetch_work_item(
                key,
                link.clone(),
                |app, _, cx| app.load_work_item_tab(cx),
                cx,
            );
        }
        // Loads what it shows as it settles (`load_work_item_tab`).
        self.open_pane_tab(
            PaneTab::WorkItem {
                key: key.to_string(),
            },
            pin,
            cx,
        );
    }

    /// "Send to agent" or "Open thread": the item opens as a preview tab
    /// beside its thread, read while the composer keeps the keyboard.
    pub(super) fn show_item_beside_thread(&mut self, key: &str, cx: &mut Context<Self>) {
        self.open_work_item_tab(key, None, false, cx);
        self.file_pane_focused = false;
    }

    /// The detail's "Open beside the chat": the Inbox closes, and the item
    /// stays in a tab beside the thread in view.
    pub(super) fn open_work_item_beside(&mut self, key: &str, cx: &mut Context<Self>) {
        self.open_work_item_tab(key, None, true, cx);
    }

    /// The tab's "Open in the Inbox": the item selected there, where it
    /// can be commented on.
    pub(super) fn open_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        self.open_inbox(false, cx);
        self.select_inbox_key(key.to_string(), cx);
    }

    /// A ticket or pull request one names (`properties.rs`), opened in the
    /// view it was named in: selected in the Inbox (fetched first when
    /// the Inbox has not got it), or in the tab.
    pub(super) fn open_related_item(
        &mut self,
        key: &str,
        view: DetailView,
        cx: &mut Context<Self>,
    ) {
        if view == DetailView::Tab {
            self.open_work_item_tab(key, None, true, cx);
            return;
        }
        if self.inbox(cx).item(key).is_some() {
            self.select_inbox_key(key.to_string(), cx);
            return;
        }
        let Some(link) = self.work_item_link(key) else {
            log::warn!("inbox: no way to fetch {key}");
            return;
        };
        let url = link.url.clone();
        self.fetch_work_item(
            key,
            link,
            move |app, key, cx| {
                if app.inbox(cx).item(key).is_some() {
                    app.select_inbox_key(key.to_string(), cx);
                } else {
                    // Without `gh` (or Backlog), the browser still shows it.
                    cx.open_url(&url);
                }
            },
            cx,
        );
    }

    /// Whether the file pane shows item `key`'s tab.
    pub(crate) fn work_item_tab_shown(&self, key: &str) -> bool {
        matches!(self.file_pane.active(), Some(PaneTab::WorkItem { key: shown }) if shown == key)
    }

    /// What the active tab's item shows, loaded; the item itself fetched
    /// first when the Inbox has not got it. As the pane settles, and when
    /// an explicit refresh dropped what was loaded.
    pub(crate) fn load_work_item_tab(&mut self, cx: &mut Context<Self>) {
        let Some(PaneTab::WorkItem { key }) = self.file_pane.active().cloned() else {
            return;
        };
        if self.inbox(cx).item(&key).is_some() {
            self.load_inbox_item(&key, cx);
            self.apply_inbox_translations(&key, cx);
            return;
        }
        if self.inbox(cx).known.get(&key).is_some() {
            return;
        }
        match self.work_item_link(&key) {
            Some(link) => {
                self.fetch_work_item(&key, link, |app, _, cx| app.load_work_item_tab(cx), cx)
            }
            None => self.inbox_state.update(cx, |inbox, cx| {
                let error = "Connect Backlog in Settings › Integrations to open this issue.";
                inbox.known.value.insert(key, Err(error.into()));
                cx.notify();
            }),
        }
    }

    /// How to fetch item `key`: its link, a Backlog key in the connected
    /// space.
    fn work_item_link(&self, key: &str) -> Option<LinkedWorkItem> {
        let space = (self.backlog.account.as_ref()).map(|account| account.space.as_str());
        LinkedWorkItem::from_item_key(key, space)
    }

    /// Fetches item `key` off the UI thread into `InboxState::known`, once
    /// at a time; `then` follows, whether it came or not.
    fn fetch_work_item(
        &mut self,
        key: &str,
        link: LinkedWorkItem,
        then: impl FnOnce(&mut Self, &str, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let started = self.inbox_mut(cx, |inbox| {
            !inbox.known.value.contains_key(key) && inbox.known.loading.insert(key.to_string())
        });
        if !started {
            return;
        }
        let cwd = self.current_cwd.clone();
        let kind = match link.kind {
            WorkItemKind::Pr => Kind::Pr,
            WorkItemKind::Issue => Kind::Issue,
        };
        let backlog = (link.host()).map(|host| (link.identifier(), host.to_string()));
        let (repo, number) = (link.repo.clone(), link.number);
        let task = cx.background_executor().spawn(async move {
            let found = match backlog {
                Some((issue, host)) => crate::backlog::issue(&issue, &host),
                None => github::work_item(std::path::Path::new(&cwd), &repo, kind, number),
            };
            found.map(|mut found| {
                found.project = cwd;
                found
            })
        });
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let found = task.await;
            if let Err(err) = &found {
                log::warn!("inbox: could not fetch {key}: {err}");
            }
            let landed = this.update(cx, |app, cx| {
                // What shows the Inbox or the tab follows it (`ui/region.rs`).
                app.inbox_state.update(cx, |inbox, cx| {
                    inbox.known.loading.remove(&key);
                    inbox.known.value.insert(key.clone(), found);
                    cx.notify();
                });
                then(app, &key, cx);
            });
            if let Err(err) = landed {
                log::debug!("inbox item fetched after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The tab's label and icon: the item's short key and title (as
    /// shown, translated or not), by kind; its key until it is known.
    pub(crate) fn work_item_tab_label(&self, key: &str, cx: &gpui::App) -> (String, IconName) {
        let inbox = self.inbox(cx);
        let Some(item) = inbox.item(key) else {
            let (name, _) = PaneTab::WorkItem { key: key.into() }.label();
            return (name, IconName::Inbox);
        };
        let name = match item.provider {
            Provider::Backlog => short_key(&item.identifier, &inbox.key_prefix).to_string(),
            Provider::GitHub => format!("#{}", item.number),
        };
        let icon = match item.kind {
            Kind::Pr => IconName::GitPullRequest,
            Kind::Issue => IconName::CircleDot,
        };
        let title = super::properties::title_shown(inbox, item);
        (format!("{name} {title}"), icon)
    }

    /// The tab's body: the item's detail, or its fetch's progress or error.
    pub(crate) fn render_work_item_tab(&self, key: &str, cx: &Context<Self>) -> AnyElement {
        let inbox = self.inbox(cx);
        if let Some(item) = inbox.item(key) {
            // One column: the pane is a share of the window, and its width
            // is not known before it is laid out.
            return self.render_item_detail(item, DetailView::Tab, 0.0, cx);
        }
        let centred = div().size_full().flex().items_center().justify_center();
        match inbox.known.get(key) {
            Some(Err(err)) => {
                let url = (self.work_item_link(key)).map(|link| link.url);
                let mut empty = EmptyState::new(
                    "work-item-error",
                    IconName::CircleAlert,
                    "Could not open this item",
                )
                .body(err.clone());
                if let Some(url) = url {
                    empty = empty.action(
                        Button::new("work-item-error-open", "Open in the browser")
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                centred.child(empty).into_any_element()
            }
            _ => {
                let muted = cx.theme().colors.fg.ink(Ink::Muted);
                centred
                    .gap_2()
                    .text_size(Text::Base.px())
                    .text_color(muted)
                    .child(loading_icon("work-item-loading", muted))
                    .child("Loading…")
                    .into_any_element()
            }
        }
    }
}
