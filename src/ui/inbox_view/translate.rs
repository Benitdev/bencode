//! An item's title and description, or one of its comments, in another
//! language (`work_items/translate.rs`), shown in place of the original
//! until the user turns back to it. A comment is translated on its own,
//! when asked, into the language last picked for the item.
//!
//! What is made is kept on disk (`translate/cache.rs`), read when the
//! Inbox first shows: a refresh or a relaunch shows it again while its
//! source text is unchanged, and the list's rows show translated titles.
//! "Always translate" (off by default) translates an item as it is opened
//! when its text is not in the language (`translate/script.rs`) and
//! nothing kept matches: one turn per item, never from the list or the
//! poll.

use std::collections::{HashMap, HashSet};

use bencode_ui::buttons::ButtonVariant;
use bencode_ui::menus::{DropdownMenu, Menu, MenuItem};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
};

use super::InboxState;
use super::blocks::cut;
use crate::app::{BenCodeApp, Surface};
use crate::github::WorkItem;
use crate::harness::catalog;
use crate::ui::app_callback::app_callback;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::tokens::{Ink, Text, Tone as _};
use crate::work_items::translate::cache::{self, Cache, Entry, hash};
use crate::work_items::translate::{self, DEFAULT_MODEL, Language, script};

pub struct Translation {
    pub language: Language,
    /// Empty for a comment.
    pub title: SharedString,
    pub body: SharedString,
    /// An item's description cut as `Described::blocks`; empty for a
    /// comment, drawn whole.
    pub blocks: Vec<SharedString>,
    /// `cache::hash` of the text it was made from; the title's is empty
    /// for a comment.
    title_hash: String,
    body_hash: String,
}

impl Translation {
    /// Made from `source` (an item's title and body, or a comment's body
    /// with no title).
    fn made(language: Language, source: (Option<&str>, &str), title: String, body: String) -> Self {
        let (source_title, source_body) = source;
        Self {
            language,
            title_hash: source_title.map(hash).unwrap_or_default(),
            body_hash: hash(source_body),
            blocks: if source_title.is_some() {
                cut(&body)
            } else {
                Vec::new()
            },
            title: title.into(),
            body: body.into(),
        }
    }

    /// As the cache kept it.
    fn cached(entry: &Entry) -> Self {
        let item = !entry.title_hash.is_empty();
        Self {
            language: entry.language,
            title: entry.title.clone().into(),
            blocks: if item { cut(&entry.body) } else { Vec::new() },
            body: entry.body.clone().into(),
            title_hash: entry.title_hash.clone(),
            body_hash: entry.body_hash.clone(),
        }
    }

    fn entry(&self, key: &str, made_at: i64) -> Entry {
        Entry {
            key: key.to_string(),
            language: self.language,
            title_hash: self.title_hash.clone(),
            title: self.title.to_string(),
            body_hash: self.body_hash.clone(),
            body: self.body.to_string(),
            made_at,
        }
    }

    /// Made from `title` (`None` for a comment) and `body` as they are
    /// now; a body not loaded (`None`) is not checked.
    fn matches(&self, title: Option<&str>, body: Option<&str>) -> bool {
        let title = match title {
            Some(title) => self.title_hash == hash(title),
            None => self.title_hash.is_empty(),
        };
        title && body.is_none_or(|body| self.body_hash == hash(body))
    }
}

/// Where the cache on disk stands.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Disk {
    #[default]
    Unread,
    Reading,
    Read,
}

/// Translations by item key or `comment_key`, for the session, over what
/// the cache on disk keeps.
#[derive(Default)]
pub struct Translations {
    /// What shows, or why it could not be made.
    value: HashMap<String, Result<Translation, String>>,
    /// What is being translated.
    running: HashSet<String>,
    /// What was turned back to its original, for the session.
    originals: HashSet<String>,
    /// The language last picked; a comment's Translate, the list's titles
    /// and "Always translate" use it. Saved.
    pub language: Language,
    /// The model every translation runs on (`harness:model`), picked in
    /// Settings; `None` is `DEFAULT_MODEL`. Saved.
    pub model: Option<String>,
    /// Opening an item translates it into `language`. Saved; off unless
    /// the user turns it on, as each costs a turn.
    pub always: bool,
    /// Every translation made, by key and language (`cache.rs`).
    cache: Cache,
    disk: Disk,
    /// The list's translated titles by item key, in `language`, made from
    /// the title as listed: kept by `refresh_translated_titles`, so a row
    /// looks one up and hashes nothing.
    titles: HashMap<String, SharedString>,
    /// Numbers the cache's saves (`cache::save`).
    saves: u64,
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

    /// Item `key`'s title as its list row shows it, when translated.
    pub fn list_title(&self, key: &str) -> Option<&SharedString> {
        self.titles
            .get(key)
            .filter(|_| !self.originals.contains(key))
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

    /// An explicit refresh tries again what failed. What was made stays,
    /// shown while the text it was made from is unchanged.
    pub(super) fn forget_errors(&mut self) {
        self.value.retain(|_, made| made.is_ok());
    }

    /// Drops `key`'s translation when made from other text than `title`
    /// and `body` (`None`: not loaded).
    fn drop_stale(&mut self, key: &str, title: Option<&str>, body: Option<&str>) {
        if matches!(self.value.get(key), Some(Ok(made)) if !made.matches(title, body)) {
            self.value.remove(key);
        }
    }

    /// Shows the kept translation of `key`'s text as it is now, in the
    /// language last picked, where nothing shows yet.
    fn show_cached(&mut self, key: &str, title: Option<&str>, body: &str) {
        if self.disk != Disk::Read || self.value.contains_key(key) {
            return;
        }
        if let Some(entry) = self.cache.valid(key, self.language, title, body) {
            let made = Translation::cached(entry);
            self.value.insert(key.to_string(), Ok(made));
        }
    }
}

impl InboxState {
    /// The list's translated titles, again: the list, the language or the
    /// kept translations changed.
    pub(super) fn refresh_translated_titles(&mut self) {
        let translations = &mut self.translations;
        let language = translations.language;
        translations.titles = (self.items.iter())
            .filter_map(|item| {
                let key = item.key();
                let title = (translations.cache).title(&key, language, &hash(&item.title))?;
                Some((key, SharedString::from(title.to_string())))
            })
            .collect();
    }

    /// Item `key`'s translations and its comments', against the text now
    /// loaded: what was made from other text no longer shows, and what the
    /// cache keeps for the text as it is shows where nothing does.
    pub(super) fn sync_translations(&mut self, key: &str) {
        let Some(title) = self.item(key).map(|item| item.title.clone()) else {
            return;
        };
        let body = match self.details.get(key) {
            Some(Ok(described)) => Some(described.details.body.as_str()),
            _ => None,
        };
        let translations = &mut self.translations;
        translations.drop_stale(key, Some(&title), body);
        if let Some(body) = body {
            translations.show_cached(key, Some(&title), body);
        }
        if let Some(Ok(thread)) = self.threads.get(key) {
            for comment in &thread.comments {
                let comment_key = comment_key(key, &comment.id);
                translations.drop_stale(&comment_key, None, Some(&comment.body));
                translations.show_cached(&comment_key, None, &comment.body);
            }
        }
    }

    /// A translation made without a model, kept as if made, for the
    /// snapshots (`app/snapshot.rs`): `source` and `translated` are a
    /// title and a description.
    #[cfg(feature = "snapshot")]
    pub(crate) fn stage_translation(
        &mut self,
        key: &str,
        language: Language,
        source: (&str, &str),
        translated: (&str, &str),
    ) {
        let made = Translation::made(
            language,
            (Some(source.0), source.1),
            translated.0.to_string(),
            translated.1.to_string(),
        );
        let translations = &mut self.translations;
        translations.language = language;
        (translations.cache).insert(made.entry(key, crate::app::now_ms()));
        translations.disk = Disk::Read;
        self.refresh_translated_titles();
        self.sync_translations(key);
    }
}

impl BenCodeApp {
    /// Reads the kept translations off the UI thread, once: as the Inbox
    /// first shows, never on the launch path.
    pub(crate) fn read_inbox_translations(&mut self, cx: &mut Context<Self>) {
        if self.inbox(cx).translations.disk != Disk::Unread {
            return;
        }
        let Some(path) = cache::path() else {
            // No data folder: translations last the session.
            self.inbox_mut(cx, |inbox| inbox.translations.disk = Disk::Read);
            return;
        };
        self.inbox_mut(cx, |inbox| inbox.translations.disk = Disk::Reading);
        let task = cx
            .background_executor()
            .spawn(async move { Cache::load(&path) });
        cx.spawn(async move |this, cx| {
            let read = task.await.unwrap_or_else(|err| {
                log::warn!("inbox: could not read translations: {err:#}");
                Cache::default()
            });
            let landed = this.update(cx, |app, cx| {
                let (made, selected) = app.inbox_state.update(cx, |inbox, cx| {
                    let translations = &mut inbox.translations;
                    // Made before the read landed: not on disk yet.
                    let made = !translations.cache.is_empty();
                    translations.cache.merge(read);
                    translations.disk = Disk::Read;
                    inbox.refresh_translated_titles();
                    let selected = inbox.selected.clone();
                    if let Some(key) = &selected {
                        inbox.sync_translations(key);
                    }
                    cx.notify();
                    (made, selected)
                });
                if made {
                    app.save_inbox_translations(cx);
                }
                if let Some(key) = selected {
                    app.auto_translate_inbox_item(&key, cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("inbox translations read after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Writes the kept translations off the UI thread, pruned. Before the
    /// read lands nothing is written: the read's landing saves.
    fn save_inbox_translations(&mut self, cx: &mut Context<Self>) {
        if self.inbox(cx).translations.disk != Disk::Read {
            return;
        }
        let Some(path) = cache::path() else {
            return;
        };
        let (entries, seq) = self.inbox_mut(cx, |inbox| {
            let translations = &mut inbox.translations;
            translations.saves += 1;
            (translations.cache.snapshot(), translations.saves)
        });
        let now = crate::app::now_ms();
        cx.background_executor()
            .spawn(async move {
                if let Err(err) = cache::save(&path, entries, seq, now) {
                    log::warn!("inbox: could not save translations: {err:#}");
                }
            })
            .detach();
    }

    /// Item `key`'s translations against its text as loaded, then "Always
    /// translate" for it: as it opens and as its loads land.
    pub(super) fn apply_inbox_translations(&mut self, key: &str, cx: &mut Context<Self>) {
        self.inbox_state.update(cx, |inbox, cx| {
            inbox.sync_translations(key);
            cx.notify();
        });
        self.auto_translate_inbox_item(key, cx);
    }

    /// "Always translate": the open item `key`, once its description is
    /// loaded and the cache read, when its title or description is not in
    /// the language and nothing made from them shows, failed or was put
    /// away for the original.
    fn auto_translate_inbox_item(&mut self, key: &str, cx: &mut Context<Self>) {
        let shown = self.work_item_tab_shown(key);
        let inbox = self.inbox(cx);
        let translations = &inbox.translations;
        let idle = translations.always
            && translations.disk == Disk::Read
            && (inbox.selected.as_deref() == Some(key) || shown)
            && !translations.value.contains_key(key)
            && !translations.running.contains(key)
            && !translations.originals.contains(key);
        if !idle {
            return;
        }
        let (Some(item), Some(Ok(described))) = (inbox.item(key), inbox.details.get(key)) else {
            return;
        };
        let language = translations.language;
        let foreign = !script::is_in(language, &item.title)
            || !script::is_in(language, &described.details.body);
        if foreign {
            self.translate_inbox_item(key, language, cx);
        }
    }

    /// "Always translate" on or off; turned on, the open item translates
    /// now.
    pub(crate) fn set_inbox_translate_always(&mut self, on: bool, cx: &mut Context<Self>) {
        self.inbox_mut(cx, |inbox| inbox.translations.always = on);
        self.save_settings(cx);
        cx.notify();
        if let Some(key) = self.inbox(cx).selected.clone() {
            self.auto_translate_inbox_item(&key, cx);
        }
    }

    /// Item `key`'s title and loaded description into `language`: shown
    /// again, taken from the cache, or made.
    pub(super) fn translate_inbox_item(
        &mut self,
        key: &str,
        language: Language,
        cx: &mut Context<Self>,
    ) {
        if self.inbox(cx).translations.language != language {
            self.inbox_mut(cx, |inbox| {
                inbox.translations.language = language;
                inbox.refresh_translated_titles();
            });
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
        let cached = match (inbox.item(key), inbox.details.get(key)) {
            _ if made => None,
            (Some(item), Some(Ok(described))) => (inbox.translations.cache)
                .valid(key, language, Some(&item.title), &described.details.body)
                .map(Translation::cached),
            _ => None,
        };
        if made || cached.is_some() {
            // Already made: show it again, no turn.
            self.inbox_state.update(cx, |inbox, cx| {
                let translations = &mut inbox.translations;
                translations.originals.remove(key);
                if let Some(cached) = cached {
                    translations.value.insert(key.to_string(), Ok(cached));
                }
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
        self.run_translation(key.to_string(), key.to_string(), cx, move || {
            translate::translate(&model, language, &title, &body).map(|(made_title, made_body)| {
                Translation::made(language, (Some(&title), &body), made_title, made_body)
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
        if let Some(entry) = translations.cache.valid(&key, language, None, body) {
            let cached = Translation::cached(entry);
            self.inbox_state.update(cx, |inbox, cx| {
                inbox.translations.originals.remove(&key);
                inbox.translations.value.insert(key, Ok(cached));
                cx.notify();
            });
            return;
        }
        let model = translations.model_key().to_string();
        let body = body.to_string();
        self.run_translation(key, item.to_string(), cx, move || {
            translate::translate_comment(&model, language, &body)
                .map(|made| Translation::made(language, (None, &body), String::new(), made))
        });
    }

    /// Runs `job` off the UI thread and keeps its translation under `key`
    /// (of item `item`), in the session and on disk.
    fn run_translation(
        &mut self,
        key: String,
        item: String,
        cx: &mut Context<Self>,
        job: impl FnOnce() -> Result<Translation, String> + Send + 'static,
    ) {
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
                let made = result.is_ok();
                app.inbox_state.update(cx, |inbox, cx| {
                    let translations = &mut inbox.translations;
                    translations.running.remove(&key);
                    if let Ok(made) = &result {
                        (translations.cache).insert(made.entry(&key, crate::app::now_ms()));
                    }
                    translations.originals.remove(&key);
                    translations.value.insert(key, result);
                    // A refresh since may have changed the text: what was
                    // made from the old shows no more, but is kept.
                    inbox.sync_translations(&item);
                    inbox.refresh_translated_titles();
                    cx.notify();
                });
                if made {
                    app.save_inbox_translations(cx);
                }
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

    pub(super) fn toggle_inbox_original(&mut self, key: &str, cx: &mut Context<Self>) {
        self.inbox_state.update(cx, |inbox, cx| {
            let originals = &mut inbox.translations.originals;
            if !originals.remove(key) {
                originals.insert(key.to_string());
            }
            cx.notify();
        });
    }

    /// The description card's Translate control: "Translated from
    /// Japanese · Show original" and the language menu while a translation
    /// shows, else the menu alone. It sits with the text it changes.
    pub(super) fn render_translate_control(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = item.key();
        let fg = cx.theme().colors.fg;
        let link = cx.theme().colors.link;
        let shown = self.inbox(cx).translations.shown(&key).is_some();
        // Read from the title: the description may be long, and drawn
        // again on every scroll.
        let from = script::written_in(&item.title).map_or_else(
            || "Translated".to_string(),
            |language| format!("Translated from {}", language.name()),
        );
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .when(shown, |el| {
                el.child(from).child("·").child(
                    div()
                        .id("inbox-show-original")
                        .cursor_pointer()
                        .text_color(link)
                        .hover(move |s| s.text_color(fg))
                        .child("Show original")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_inbox_original(&key, cx)),
                        ),
                )
            })
            .child(self.render_translate_menu(item, cx))
    }

    /// Translate, as a menu of languages, and the way back to the original.
    fn render_translate_menu(
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
        let always = MenuItem::check(
            format!("Always translate into {}", translations.language.label()),
            translations.always,
        )
        .on_click(app_callback(cx, |this, cx| {
            let on = !this.inbox(cx).translations.always;
            this.set_inbox_translate_always(on, cx)
        }));
        // Where the model is picked.
        let model = catalog::label_for(translations.model_key());
        let menu = menu.separator().item(always).item(
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
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .icon(IconName::Languages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Comment, Details, Thread};
    use crate::work_items::Provider;

    const KEY: &str = "backlog:a-1";

    fn item(title: &str) -> WorkItem {
        WorkItem {
            provider: Provider::Backlog,
            identifier: "A-1".into(),
            title: title.into(),
            ..Default::default()
        }
    }

    fn loaded(inbox: &mut InboxState, body: &str, comment: &str) {
        let details = Details {
            body: body.into(),
            ..Default::default()
        };
        (inbox.details.value).insert(KEY.into(), Ok(details.into()));
        let thread = Thread {
            comments: vec![Comment {
                id: "7".into(),
                body: comment.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        inbox.threads.value.insert(KEY.into(), Ok(thread));
    }

    /// An item open with a translation of its title, body and comment
    /// kept, as if read from disk.
    fn translated() -> InboxState {
        let mut inbox = InboxState::default();
        inbox.translations.disk = Disk::Read;
        inbox.selected = Some(KEY.into());
        inbox.set_items(vec![item("件名")]);
        loaded(&mut inbox, "本文", "了解");
        let vi = Language::Vietnamese;
        let made = Translation::made(
            vi,
            (Some("件名"), "本文"),
            "Tiêu đề".into(),
            "Nội dung".into(),
        );
        let comment = Translation::made(vi, (None, "了解"), String::new(), "Đã rõ".into());
        let translations = &mut inbox.translations;
        translations.cache.insert(made.entry(KEY, 1));
        translations
            .cache
            .insert(comment.entry(&comment_key(KEY, "7"), 1));
        inbox.refresh_translated_titles();
        inbox.sync_translations(KEY);
        inbox
    }

    #[test]
    fn kept_translations_show_for_the_text_they_were_made_from() {
        let inbox = translated();
        let translations = &inbox.translations;
        assert_eq!(
            translations.shown(KEY).map(|t| t.title.as_ref()),
            Some("Tiêu đề")
        );
        assert!(!translations.shown(KEY).unwrap().blocks.is_empty());
        assert_eq!(
            translations.shown_comment(KEY, "7").map(|b| b.as_ref()),
            Some("Đã rõ")
        );
        assert_eq!(
            translations.list_title(KEY).map(|t| t.as_ref()),
            Some("Tiêu đề")
        );
    }

    #[test]
    fn a_refresh_keeps_translations_while_the_text_is_unchanged() {
        let mut inbox = translated();
        inbox
            .translations
            .value
            .insert("backlog:a-2".into(), Err("failed".into()));
        inbox.forget_item_loads();
        assert!(
            inbox.translations.error("backlog:a-2").is_none(),
            "a failure is retried"
        );
        inbox.set_items(vec![item("件名")]);
        loaded(&mut inbox, "本文", "了解");
        inbox.sync_translations(KEY);
        assert!(inbox.translations.shown(KEY).is_some());
        assert!(inbox.translations.shown_comment(KEY, "7").is_some());
        assert!(inbox.translations.list_title(KEY).is_some());
    }

    #[test]
    fn a_changed_source_hides_only_what_it_invalidates() {
        // The description and the comment changed, the title did not.
        let mut inbox = translated();
        inbox.forget_item_loads();
        loaded(&mut inbox, "本文を更新", "了解です");
        inbox.sync_translations(KEY);
        assert!(inbox.translations.shown(KEY).is_none());
        assert!(inbox.translations.shown_comment(KEY, "7").is_none());
        assert!(
            inbox.translations.list_title(KEY).is_some(),
            "the title still matches"
        );
        // The title changed too: the list shows the original.
        inbox.set_items(vec![item("新しい件名")]);
        assert!(inbox.translations.list_title(KEY).is_none());
    }

    #[test]
    fn the_original_hides_the_list_title_too() {
        let mut inbox = translated();
        inbox.translations.originals.insert(KEY.into());
        assert!(inbox.translations.shown(KEY).is_none());
        assert!(inbox.translations.list_title(KEY).is_none());
    }
}
