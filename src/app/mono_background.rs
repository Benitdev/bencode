//! MonoCode `monoBackground.ts`: a Mono's own chat background. Only the
//! image is the user's choice; it always shows under the Haze, dimmed the
//! same on an empty chat as in a running one. The copy is kept in
//! BenCode's `backgrounds` folder as `mono-<id>.<ext>`, and its path on the
//! Mono (`settings.json`); a Mono without one shows the app's.

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

fn stem(mono: &str) -> String {
    format!("mono-{mono}")
}

/// Each Mono's image as drawn, by Mono, with what it was made from.
#[derive(Default)]
pub struct MonoBackgrounds {
    images: HashMap<String, ((String, bool), Arc<RenderImage>)>,
    loading: HashMap<String, (String, bool)>,
    /// The Mono whose background dialog is open.
    pub dialog: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
}

impl BenCodeApp {
    /// The image behind a Mono's chat, if it has one of its own.
    pub fn mono_background_for(&self, session_id: &str) -> Option<(Arc<RenderImage>, f32)> {
        let mono = self.mono_for_session(session_id)?;
        Some((self.mono_background_image(&mono.id)?, CHAT_BACKGROUND_OPACITY_DEFAULT))
    }

    /// A Mono's own image as drawn, if it has one and it is ready.
    pub fn mono_background_image(&self, mono: &str) -> Option<Arc<RenderImage>> {
        self.mono(mono)?.background.as_ref()?;
        let (_, image) = self.mono_backgrounds.images.get(mono)?;
        Some(image.clone())
    }

    /// The image of the Mono in view, and of the one whose dialog is open,
    /// drawn when it is missing or stale (called each frame, so it only
    /// compares unless something changed).
    pub fn sync_mono_background(&mut self, light: bool, cx: &mut Context<Self>) {
        let mut wanted: Vec<(String, String)> = Vec::new();
        let view = self.mono_in_view().map(|mono| mono.id.clone());
        for id in view.into_iter().chain(self.mono_backgrounds.dialog.clone()) {
            if let Some(path) = self.mono(&id).and_then(|mono| mono.background.clone())
                && !wanted.iter().any(|(seen, _)| *seen == id)
            {
                wanted.push((id, path));
            }
        }
        for (id, path) in wanted {
            self.draw_mono_background(id, path, light, cx);
        }
    }

    fn draw_mono_background(&mut self, id: String, path: String, light: bool, cx: &mut Context<Self>) {
        let key = (path.clone(), light && EFFECT.follows_theme());
        let state = &mut self.mono_backgrounds;
        if state.images.get(&id).is_some_and(|(made, _)| *made == key)
            || state.loading.get(&id) == Some(&key)
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
                if this.mono_backgrounds.loading.get(&id) != Some(&key) {
                    return;
                }
                this.mono_backgrounds.loading.remove(&id);
                match result {
                    Ok(image) => {
                        if let Some((_, old)) = this.mono_backgrounds.images.insert(id, (key, image)) {
                            cx.drop_image(old, None);
                        }
                    }
                    Err(err) => {
                        log::warn!("a Mono's background: {err:#}");
                        this.mono_backgrounds.error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("Mono background after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The rail menu's "Background image".
    pub fn open_mono_background(&mut self, mono: &str, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.mono_backgrounds.dialog = Some(mono.to_string());
        self.mono_backgrounds.error = None;
        cx.notify();
    }

    pub fn close_mono_background(&mut self, cx: &mut Context<Self>) {
        self.mono_backgrounds.dialog = None;
        cx.notify();
    }

    /// Choose image…: the file copied in, then shown.
    pub fn choose_mono_background(&mut self, mono: &str, cx: &mut Context<Self>) {
        if self.mono_backgrounds.busy {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose background image".into()),
        });
        let mono = mono.to_string();
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
                this.mono_backgrounds.busy = true;
                this.mono_backgrounds.error = None;
                cx.notify();
            });
            if busy.is_err() {
                return;
            }
            let name = stem(&mono);
            let saved = cx
                .background_executor()
                .spawn(async move { save_background_named(&backgrounds_dir()?, &file, &name) })
                .await;
            let landed = this.update(cx, |this, cx| {
                this.mono_backgrounds.busy = false;
                match saved {
                    Ok(path) => {
                        // The same name may hold a new picture: draw again.
                        if let Some((_, old)) = this.mono_backgrounds.images.remove(&mono) {
                            cx.drop_image(old, None);
                        }
                        let path = path.to_string_lossy().into_owned();
                        this.update_monos(
                            |roster| {
                                if let Some(entry) = roster.iter_mut().find(|m| m.id == mono) {
                                    entry.background = Some(path);
                                }
                            },
                            cx,
                        );
                    }
                    Err(err) => this.mono_backgrounds.error = Some(format!("{err:#}")),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("Mono background saved after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `removeMonoBackground`: forgotten at once, the copy removed
    /// behind. Also when the Mono is deleted.
    pub fn remove_mono_background(&mut self, mono: &str, cx: &mut Context<Self>) {
        if let Some((_, old)) = self.mono_backgrounds.images.remove(mono) {
            cx.drop_image(old, None);
        }
        let had = self.mono(mono).is_some_and(|m| m.background.is_some());
        if had {
            let id = mono.to_string();
            self.update_monos(
                |roster| {
                    if let Some(entry) = roster.iter_mut().find(|m| m.id == id) {
                        entry.background = None;
                    }
                },
                cx,
            );
        }
        let name = stem(mono);
        cx.background_executor()
            .spawn(async move {
                if let Err(err) = backgrounds_dir().and_then(|dir| remove_backgrounds_named(&dir, &name)) {
                    log::warn!("removing a Mono's background: {err:#}");
                }
            })
            .detach();
        cx.notify();
    }
}
