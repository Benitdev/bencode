//! An item's title and description, or one of its comments, in another
//! language (`work_items/translate.rs`), shown in place of the original
//! until the user turns back to it. A comment is translated on its own,
//! when asked, into the language last picked for the item.

use std::collections::{HashMap, HashSet};

use bencode_ui::buttons::ButtonVariant;
use bencode_ui::menus::{DropdownMenu, Menu, MenuItem};
use bencode_ui::primitives::IconName;
use gpui::{Context, IntoElement, SharedString};

use super::blocks::cut;
use crate::app::{BenCodeApp, Surface};
use crate::github::WorkItem;
use crate::harness::catalog;
use crate::ui::app_callback::app_callback;
use crate::ui::settings_modal::SettingsTab;
use crate::work_items::translate::{self, DEFAULT_MODEL, Language};

pub struct Translation {
    pub language: Language,
    /// Empty for a comment.
    pub title: SharedString,
    pub body: SharedString,
    /// An item's description cut as `Described::blocks`; empty for a
    /// comment, drawn whole.
    pub blocks: Vec<SharedString>,
}

/// Translations by item key or `comment_key`, while the Inbox's list
/// stays as loaded.
#[derive(Default)]
pub struct Translations {
    value: HashMap<String, Result<Translation, String>>,
    /// What is being translated.
    running: HashSet<String>,
    /// What was turned back to its original.
    originals: HashSet<String>,
    /// The language last picked; a comment's Translate uses it. Saved.
    pub language: Language,
    /// The model every translation runs on (`harness:model`), picked in
    /// Settings; `None` is `DEFAULT_MODEL`. Saved.
    pub model: Option<String>,
}

/// Where comment `id` of item `item` keeps its translation.
pub fn comment_key(item: &str, id: &str) -> String {
    format!("{item}#comment-{id}")
}

impl Translations {
    pub fn model_key(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    /// The translation item `key` shows instead of its original.
    pub fn shown(&self, key: &str) -> Option<&Translation> {
        (self.value.get(key)?.as_ref().ok()).filter(|_| !self.originals.contains(key))
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        self.value.get(key)?.as_ref().err().map(String::as_str)
    }

    pub fn is_running(&self, key: &str) -> bool {
        self.running.contains(key)
    }

    pub fn is_made(&self, key: &str) -> bool {
        matches!(self.value.get(key), Some(Ok(_)))
    }

    /// Comment `id`'s body as shown, when translated.
    pub fn shown_comment(&self, item: &str, id: &str) -> Option<&SharedString> {
        self.shown(&comment_key(item, id)).map(|t| &t.body)
    }

    /// A refresh reloads the descriptions these were made from.
    pub(super) fn clear(&mut self) {
        self.value.clear();
        self.originals.clear();
    }
}

impl BenCodeApp {
    /// Item `key`'s title and loaded description into `language`.
    fn translate_inbox_item(&mut self, key: &str, language: Language, cx: &mut Context<Self>) {
        if self.inbox(cx).translations.language != language {
            self.inbox_mut(cx, |inbox| inbox.translations.language = language);
            self.save_settings(cx);
        }
        let inbox = self.inbox(cx);
        if inbox.translations.is_running(key) {
            return;
        }
        let made = matches!(
            inbox.translations.value.get(key),
            Some(Ok(done)) if done.language == language
        );
        if made {
            // Already made: show it again.
            self.inbox_state.update(cx, |inbox, cx| {
                inbox.translations.originals.remove(key);
                cx.notify();
            });
            return;
        }
        let inbox = self.inbox(cx);
        let (Some(item), Some(Ok(described))) = (inbox.item(key), inbox.details.get(key)) else {
            return;
        };
        let (title, body) = (item.title.clone(), described.details.body.clone());
        let model = inbox.translations.model_key().to_string();
        self.run_translation(key.to_string(), cx, move || {
            translate::translate(&model, language, &title, &body).map(|(title, body)| Translation {
                language,
                title: title.into(),
                blocks: cut(&body),
                body: body.into(),
            })
        });
    }

    /// A comment's Translate: its translation made, shown or put away.
    pub(super) fn translate_inbox_comment(
        &mut self,
        item: &str,
        id: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) {
        let key = comment_key(item, id);
        let translations = &self.inbox(cx).translations;
        if translations.is_running(&key) {
            return;
        }
        if translations.is_made(&key) {
            self.toggle_inbox_original(&key, cx);
            return;
        }
        let language = translations.language;
        let model = translations.model_key().to_string();
        let body = body.to_string();
        self.run_translation(key, cx, move || {
            translate::translate_comment(&model, language, &body).map(|body| Translation {
                language,
                title: SharedString::default(),
                body: body.into(),
                blocks: Vec::new(),
            })
        });
    }

    /// Runs `job` off the UI thread and keeps its translation under `key`.
    fn run_translation(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
        job: impl FnOnce() -> Result<Translation, String> + Send + 'static,
    ) {
        let generation = self.inbox(cx).generation;
        self.inbox_state.update(cx, |inbox, cx| {
            inbox.translations.running.insert(key.clone());
            cx.notify();
        });
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(err) = &result {
                log::warn!("inbox: could not translate {key}: {err}");
            }
            let landed = this.update(cx, |app, cx| {
                app.inbox_state.update(cx, |inbox, cx| {
                    inbox.translations.running.remove(&key);
                    // A refresh since may have changed the text.
                    if inbox.generation == generation {
                        inbox.translations.originals.remove(&key);
                        inbox.translations.value.insert(key, result);
                    }
                    cx.notify();
                });
            });
            if let Err(err) = landed {
                log::debug!("inbox translation after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The model later translations run on; those made stay.
    pub(crate) fn set_translate_model(&mut self, key: &str, cx: &mut Context<Self>) {
        let model = Some(key.to_string()).filter(|key| key != DEFAULT_MODEL);
        self.inbox_mut(cx, |inbox| inbox.translations.model = model);
        self.save_settings(cx);
        cx.notify();
    }

    fn toggle_inbox_original(&mut self, key: &str, cx: &mut Context<Self>) {
        self.inbox_state.update(cx, |inbox, cx| {
            let originals = &mut inbox.translations.originals;
            if !originals.remove(key) {
                originals.insert(key.to_string());
            }
            cx.notify();
        });
    }

    /// Translate, as a menu of languages, and the way back to the original.
    pub(super) fn render_translate_menu(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = item.key();
        let inbox = self.inbox(cx);
        let translations = &inbox.translations;
        let loaded = matches!(inbox.details.get(&key), Some(Ok(_)));
        let busy = translations.is_running(&key);
        let shown = translations.shown(&key).map(|t| t.language);
        let menu = Language::ALL.iter().fold(Menu::new(), |menu, &language| {
            let key = key.clone();
            menu.item(
                MenuItem::check(language.label(), shown == Some(language))
                    .disabled(busy || !loaded)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.translate_inbox_item(&key, language, cx)
                    })),
            )
        });
        let menu = if translations.is_made(&key) {
            let key = key.clone();
            let label = if shown.is_some() {
                "Show original"
            } else {
                "Show translation"
            };
            menu.separator().item(
                MenuItem::new(label).on_click(app_callback(cx, move |this, cx| {
                    this.toggle_inbox_original(&key, cx)
                })),
            )
        } else {
            menu
        };
        // Where the model is picked.
        let model = catalog::label_for(translations.model_key());
        let menu = menu.separator().item(
            MenuItem::new(format!("Model: {model}…"))
                .icon(IconName::Settings)
                .on_click(app_callback(cx, |this, cx| {
                    this.select_settings_tab(SettingsTab::Integrations, cx);
                    this.show_surface(Surface::Settings, cx);
                })),
        );
        let label = match shown {
            _ if busy => "Translating…".to_string(),
            Some(language) => language.label().to_string(),
            None => "Translate".to_string(),
        };
        DropdownMenu::new("inbox-translate", label, menu)
            .variant(ButtonVariant::Secondary)
            .icon(IconName::Languages)
    }
}
