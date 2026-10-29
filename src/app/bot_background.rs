//! MonoCode `monoBackground.ts`: a Bot's own chat background. Only the
//! image is the user's choice; it always shows under the Haze, dimmed the
//! same on an empty chat as in a running one. The copy is kept in
//! BenCode's `backgrounds` folder as `mono-<id>.<ext>`, and its path on the
//! Bot (`settings.json`); a Bot without one shows the app's.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui::{Context, PathPromptOptions, RenderImage};

use super::chat_background::{
    backgrounds_dir, decode, remove_backgrounds_named, render_image, save_background_named,
};
use crate::app::BenCodeApp;
use crate::ui::appearance::{BackgroundEffect, CHAT_BACKGROUND_OPACITY_DEFAULT};

/// MonoCode's locked look: `gradient-blur`.
const EFFECT: BackgroundEffect = BackgroundEffect::GradientBlur;

fn stem(bot: &str) -> String {
    format!("mono-{bot}")
}

/// Each Bot's image as drawn, by Bot, with what it was made from.
#[derive(Default)]
pub struct BotBackgrounds {
    images: HashMap<String, ((String, bool), Arc<RenderImage>)>,
    loading: HashMap<String, (String, bool)>,
    /// What could not be drawn, so it is not tried again every frame.
    failed: HashMap<String, (String, bool)>,
    /// The Bot whose background dialog is open.
    pub dialog: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
}

impl BenCodeApp {
    /// The image behind a Bot's chat, if it has one of its own.
    pub fn bot_background_for(&self, session_id: &str) -> Option<(Arc<RenderImage>, f32)> {
        let bot = self.bot_for_session(session_id)?;
        Some((
            self.bot_background_image(&bot.id)?,
            CHAT_BACKGROUND_OPACITY_DEFAULT,
        ))
    }

    /// A Bot's own image as drawn, if it has one and it is ready.
    pub fn bot_background_image(&self, bot: &str) -> Option<Arc<RenderImage>> {
        self.bot(bot)?.background.as_ref()?;
        let (_, image) = self.bot_backgrounds.images.get(bot)?;
        Some(image.clone())
    }

    /// The image of the Bot in view, and of the one whose dialog is open,
    /// drawn when it is missing or stale (called each frame, so it only
    /// compares unless something changed).
    pub fn sync_bot_background(&mut self, light: bool, cx: &mut Context<Self>) {
        let mut wanted: Vec<(String, String)> = Vec::new();
        let view = self.bot_in_view().map(|bot| bot.id.clone());
        for id in view.into_iter().chain(self.bot_backgrounds.dialog.clone()) {
            if let Some(path) = self.bot(&id).and_then(|bot| bot.background.clone())
                && !wanted.iter().any(|(seen, _)| *seen == id)
            {
                wanted.push((id, path));
            }
        }
        // A Bot out of view gives its image back: each is up to 16 MB.
        let unwanted: Vec<String> = (self.bot_backgrounds.images.keys())
            .filter(|id| !wanted.iter().any(|(want, _)| want == *id))
            .cloned()
            .collect();
        for id in unwanted {
            if let Some((_, image)) = self.bot_backgrounds.images.remove(&id) {
                cx.drop_image(image, None);
            }
        }
        for (id, path) in wanted {
            self.draw_bot_background(id, path, light, cx);
        }
    }

    fn draw_bot_background(
        &mut self,
        id: String,
        path: String,
        light: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (path.clone(), light && EFFECT.follows_theme());
        let state = &mut self.bot_backgrounds;
        if state.images.get(&id).is_some_and(|(made, _)| *made == key)
            || state.loading.get(&id) == Some(&key)
            || state.failed.get(&id) == Some(&key)
        {
            return;
        }
        state.loading.insert(id.clone(), key.clone());
        let job = key.clone();
        let task = cx.background_executor().spawn(async move {
            let source = decode(Path::new(&job.0))?;
            anyhow::Ok(Arc::new(render_image(&source, EFFECT, job.1)?))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                if this.bot_backgrounds.loading.get(&id) != Some(&key) {
                    return;
                }
                this.bot_backgrounds.loading.remove(&id);
                match result {
                    Ok(image) => {
                        this.bot_backgrounds.failed.remove(&id);
                        if let Some((_, old)) = this.bot_backgrounds.images.insert(id, (key, image))
                        {
                            cx.drop_image(old, None);
                        }
                    }
                    Err(err) => {
                        log::warn!("a bot's background: {err:#}");
                        this.bot_backgrounds.error = Some(format!("{err:#}"));
                        this.bot_backgrounds.failed.insert(id, key);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("bot background after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The rail menu's "Background image".
    pub fn open_bot_background(&mut self, bot: &str, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.bot_backgrounds.dialog = Some(bot.to_string());
        self.bot_backgrounds.error = None;
        cx.notify();
    }

    pub fn close_bot_background(&mut self, cx: &mut Context<Self>) {
        self.bot_backgrounds.dialog = None;
        cx.notify();
    }

    /// Choose image…: the file copied in, then shown.
    pub fn choose_bot_background(&mut self, bot: &str, cx: &mut Context<Self>) {
        if self.bot_backgrounds.busy {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose background image".into()),
        });
        let bot = bot.to_string();
        cx.spawn(async move |this, cx| {
            let file = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(err)) => {
                    log::error!("background picker failed: {err:#}");
                    None
                }
            };
            let Some(file) = file else {
                return;
            };
            let busy = this.update(cx, |this, cx| {
                this.bot_backgrounds.busy = true;
                this.bot_backgrounds.error = None;
                cx.notify();
            });
            if busy.is_err() {
                return;
            }
            let name = stem(&bot);
            let saved = cx
                .background_executor()
                .spawn(async move { save_background_named(&backgrounds_dir()?, &file, &name) })
                .await;
            let landed = this.update(cx, |this, cx| {
                this.bot_backgrounds.busy = false;
                match saved {
                    Ok(path) => {
                        // The same name may hold a new picture: draw again.
                        this.bot_backgrounds.failed.remove(&bot);
                        if let Some((_, old)) = this.bot_backgrounds.images.remove(&bot) {
                            cx.drop_image(old, None);
                        }
                        let path = path.to_string_lossy().into_owned();
                        this.update_bots(
                            |roster| {
                                if let Some(entry) = roster.iter_mut().find(|m| m.id == bot) {
                                    entry.background = Some(path);
                                }
                            },
                            cx,
                        );
                    }
                    Err(err) => this.bot_backgrounds.error = Some(format!("{err:#}")),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("bot background saved after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `removeMonoBackground`: forgotten at once, the copy removed
    /// behind. Also when the Bot is deleted.
    pub fn remove_bot_background(&mut self, bot: &str, cx: &mut Context<Self>) {
        if let Some((_, old)) = self.bot_backgrounds.images.remove(bot) {
            cx.drop_image(old, None);
        }
        let had = self.bot(bot).is_some_and(|m| m.background.is_some());
        if had {
            let id = bot.to_string();
            self.update_bots(
                |roster| {
                    if let Some(entry) = roster.iter_mut().find(|m| m.id == id) {
                        entry.background = None;
                    }
                },
                cx,
            );
        }
        let name = stem(bot);
        cx.background_executor()
            .spawn(async move {
                if let Err(err) =
                    backgrounds_dir().and_then(|dir| remove_backgrounds_named(&dir, &name))
                {
                    log::warn!("removing a bot's background: {err:#}");
                }
            })
            .detach();
        cx.notify();
    }
}
