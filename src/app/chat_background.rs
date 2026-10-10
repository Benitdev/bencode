//! MonoCode chat background (`src-tauri/src/chat_background.rs`,
//! `features/projects/model/chatBackground.ts`,
//! `settings/model/newThreadBackgroundEffects.ts`): the image kept behind
//! the chat panes. The chosen file is copied into BenCode's own
//! `backgrounds` folder, decoded and redrawn under the chosen effect on the
//! background executor, and held as one `RenderImage` for the panes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use gpui::{Context, PathPromptOptions, RenderImage};

use crate::app::BenCodeApp;
use crate::ui::appearance::{self, BackgroundEffect, ChatBackgroundScope};
use crate::ui::background_effects::{self, Source};

const MAX_BACKGROUND_BYTES: u64 = 25 * 1024 * 1024;
const ALLOWED_EXT: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
const FILE_STEM: &str = "chat-background";

/// What an image on screen was made from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageKey {
    path: String,
    revision: u64,
    effect: BackgroundEffect,
    /// The theme, for the effects that follow it; `false` for the others.
    light: bool,
}

/// The background as drawn; what the user chose is in
/// `AppearancePrefs::chat_background`.
#[derive(Default)]
pub struct ChatBackground {
    /// Shown until its replacement is ready, so a change of effect does
    /// not blink.
    image: Option<Arc<RenderImage>>,
    shown: Option<ImageKey>,
    loading: Option<ImageKey>,
    generation: u64,
    /// A file is being copied or removed.
    pub busy: bool,
    pub error: Option<String>,
}

impl ChatBackground {
    pub fn image(&self) -> Option<&Arc<RenderImage>> {
        self.image.as_ref()
    }
}

/// BenCode's `backgrounds` folder, beside `settings.json`.
fn backgrounds_dir() -> Result<PathBuf> {
    let dir = crate::settings::settings_dir()
        .context("no home directory")?
        .join("backgrounds");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// MonoCode `background_extension`.
fn background_extension(source: &Path) -> Result<String> {
    let ext = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ALLOWED_EXT.contains(&ext.as_str()) {
        Ok(ext)
    } else {
        bail!("Background must be a PNG, JPG, GIF, or WebP image.")
    }
}

/// MonoCode `remove_existing_backgrounds`.
fn remove_existing_backgrounds(dir: &Path) -> Result<()> {
    let prefix = format!("{FILE_STEM}.");
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == FILE_STEM || name.starts_with(&prefix) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("removing {}", entry.path().display()));
                }
            }
        }
    }
    Ok(())
}

/// MonoCode `save_chat_background_sync`: the saved copy's path.
fn save_background(dir: &Path, source: &Path) -> Result<PathBuf> {
    let meta = std::fs::metadata(source).with_context(|| source.display().to_string())?;
    if !meta.is_file() {
        bail!("Not a file");
    }
    if meta.len() > MAX_BACKGROUND_BYTES {
        bail!(
            "Background is too large (maximum {} MB).",
            MAX_BACKGROUND_BYTES / 1024 / 1024
        );
    }
    let ext = background_extension(source)?;
    let dest = dir.join(format!("{FILE_STEM}.{ext}"));
    let temp = dir.join(format!(".{FILE_STEM}-upload"));
    // Copy first so choosing the currently saved image remains safe.
    std::fs::copy(source, &temp).with_context(|| temp.display().to_string())?;
    remove_existing_backgrounds(dir)?;
    std::fs::rename(&temp, &dest).with_context(|| dest.display().to_string())?;
    Ok(dest)
}

fn decode(path: &Path) -> Result<Source> {
    let image = image::ImageReader::open(path)
        .with_context(|| path.display().to_string())?
        .with_guessed_format()?
        .decode()
        .context("Unable to read the background image.")?;
    Ok(Source::new(image.into_rgba8()))
}

/// The artwork under `effect` as GPUI draws it (BGRA).
fn render_image(source: &Source, effect: BackgroundEffect, light: bool) -> Result<RenderImage> {
    let mut pixels = background_effects::render(source, effect, light);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(source.width, source.height, pixels)
        .context("The background effect returned no image.")?;
    Ok(RenderImage::new(vec![image::Frame::new(buffer)]))
}

impl BenCodeApp {
    /// The image a pane lays behind its content and how strongly: the empty
    /// or the session visibility, or nothing where the scope leaves it out.
    pub fn chat_background_for(&self, session_empty: bool) -> Option<(Arc<RenderImage>, f32)> {
        let prefs = &self.appearance.chat_background;
        prefs.path.as_ref()?;
        let opacity = if session_empty {
            prefs.empty_opacity
        } else if prefs.scope == ChatBackgroundScope::Empty {
            return None;
        } else {
            prefs.session_opacity
        };
        Some((self.chat_background.image.clone()?, opacity))
    }

    /// Starts drawing the background when what is shown no longer matches
    /// the choices (image, effect, theme). Called each frame, so it only
    /// compares unless something changed.
    pub fn sync_chat_background(&mut self, light: bool, cx: &mut Context<Self>) {
        let prefs = &self.appearance.chat_background;
        let Some(path) = prefs.path.as_deref() else {
            if self.chat_background.shown.is_some() || self.chat_background.loading.is_some() {
                self.drop_chat_background_image(cx);
            }
            return;
        };
        let light = light && prefs.effect.follows_theme();
        let made_from = |key: &Option<ImageKey>| {
            key.as_ref().is_some_and(|key| {
                key.path == path
                    && key.revision == prefs.revision
                    && key.effect == prefs.effect
                    && key.light == light
            })
        };
        if made_from(&self.chat_background.shown) || made_from(&self.chat_background.loading) {
            return;
        }
        let key = ImageKey {
            path: path.to_string(),
            revision: prefs.revision,
            effect: prefs.effect,
            light,
        };
        let state = &mut self.chat_background;
        state.loading = Some(key.clone());
        state.generation += 1;
        let generation = state.generation;
        let job = key.clone();
        // The decoded artwork is not kept: at up to 2048px it would hold
        // 16MB for the rare change of effect.
        let task = cx.background_executor().spawn(async move {
            let source = decode(Path::new(&job.path))?;
            anyhow::Ok(Arc::new(render_image(&source, job.effect, job.light)?))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                if this.chat_background.generation != generation {
                    return;
                }
                this.chat_background.loading = None;
                let image = match result {
                    Ok(image) => Some(image),
                    Err(err) => {
                        log::warn!("chat background {}: {err:#}", key.path);
                        this.chat_background.error = Some(format!("{err:#}"));
                        None
                    }
                };
                if let Some(old) = std::mem::replace(&mut this.chat_background.image, image) {
                    cx.drop_image(old, None);
                }
                // Also after a failure, so the same image is not tried
                // again every frame.
                this.chat_background.shown = Some(key);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("chat background ready after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn drop_chat_background_image(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.chat_background;
        state.generation += 1;
        state.shown = None;
        state.loading = None;
        if let Some(old) = state.image.take() {
            cx.drop_image(old, None);
        }
    }

    /// MonoCode `onChooseChatBackground` (`pickAndSaveChatBackground`).
    pub fn choose_chat_background(&mut self, cx: &mut Context<Self>) {
        if self.chat_background.busy {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose chat background".into()),
        });
        cx.spawn(async move |this, cx| {
            let file = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None, // dismissed
                Ok(Err(err)) => {
                    log::error!("background picker failed: {err:#}");
                    None
                }
            };
            let Some(file) = file else {
                return;
            };
            let started = this.update(cx, |this, cx| {
                this.chat_background.busy = true;
                this.chat_background.error = None;
                cx.notify();
            });
            if let Err(err) = started {
                log::debug!("background picked after app drop: {err:#}");
                return;
            }
            let saved = cx
                .background_executor()
                .spawn(async move { save_background(&backgrounds_dir()?, &file) })
                .await;
            let landed = this.update(cx, |this, cx| {
                this.chat_background.busy = false;
                match saved {
                    Ok(path) => {
                        let prefs = &mut this.appearance.chat_background;
                        prefs.path = Some(path.to_string_lossy().into_owned());
                        prefs.revision += 1;
                        this.save_settings(cx);
                    }
                    Err(err) => {
                        log::warn!("saving the chat background: {err:#}");
                        this.chat_background.error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("background saved after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `onClearChatBackground`: forgets the image at once and
    /// removes the saved copy behind.
    pub fn clear_chat_background(&mut self, cx: &mut Context<Self>) {
        if self.chat_background.busy {
            return;
        }
        self.appearance.chat_background.path = None;
        self.chat_background.error = None;
        self.chat_background.busy = true;
        self.drop_chat_background_image(cx);
        self.save_settings(cx);
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { remove_existing_backgrounds(&backgrounds_dir()?) });
        cx.spawn(async move |this, cx| {
            let removed = task.await;
            let landed = this.update(cx, |this, cx| {
                this.chat_background.busy = false;
                if let Err(err) = removed {
                    log::warn!("removing the chat background: {err:#}");
                    this.chat_background.error = Some(format!("{err:#}"));
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("background removed after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `onNewThreadBackgroundEffect`.
    pub fn set_background_effect(&mut self, effect: BackgroundEffect, cx: &mut Context<Self>) {
        self.appearance.chat_background.effect = effect;
        self.chat_background.error = None;
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode `onChatBackgroundScope`.
    pub fn set_chat_background_scope(
        &mut self,
        scope: ChatBackgroundScope,
        cx: &mut Context<Self>,
    ) {
        self.appearance.chat_background.scope = scope;
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode `onChatBackgroundEmptyOpacity` / `SessionOpacity`.
    pub fn set_chat_background_opacity(
        &mut self,
        empty: bool,
        opacity: f32,
        cx: &mut Context<Self>,
    ) {
        let opacity = appearance::clamp_background_opacity(opacity);
        let prefs = &mut self.appearance.chat_background;
        let slot = if empty {
            &mut prefs.empty_opacity
        } else {
            &mut prefs.session_opacity
        };
        if *slot == opacity {
            return;
        }
        *slot = opacity;
        self.save_settings(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bencode-background-{name}-{}", std::process::id()));
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_supported_extensions_case_insensitively() {
        assert_eq!(
            background_extension(Path::new("wallpaper.JPEG")).unwrap(),
            "jpeg"
        );
        assert_eq!(
            background_extension(Path::new("wallpaper.webp")).unwrap(),
            "webp"
        );
        assert!(background_extension(Path::new("wallpaper.txt")).is_err());
        assert!(background_extension(Path::new("wallpaper")).is_err());
    }

    #[test]
    fn saving_replaces_the_previous_background() {
        let dir = temp_dir("save");
        let store = dir.join("backgrounds");
        std::fs::create_dir_all(&store).unwrap();
        let first = dir.join("one.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .save(&first)
            .unwrap();
        let saved = save_background(&store, &first).unwrap();
        assert_eq!(saved, store.join("chat-background.png"));

        let second = dir.join("two.JPG");
        std::fs::copy(&first, &second).unwrap();
        let saved = save_background(&store, &second).unwrap();
        assert_eq!(saved, store.join("chat-background.jpg"));
        let names: Vec<_> = std::fs::read_dir(&store)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["chat-background.jpg"]);

        // Choosing the saved copy itself keeps it.
        let again = save_background(&store, &saved).unwrap();
        assert!(again.is_file());

        assert!(save_background(&store, &dir.join("missing.png")).is_err());
        remove_existing_backgrounds(&store).unwrap();
        assert_eq!(std::fs::read_dir(&store).unwrap().count(), 0);
    }

    #[test]
    fn decoded_artwork_renders_as_bgra() {
        let dir = temp_dir("decode");
        let file = dir.join("art.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .save(&file)
            .unwrap();
        let source = decode(&file).unwrap();
        assert_eq!((source.width, source.height), (4, 4));
        let image = render_image(&source, BackgroundEffect::None, false).unwrap();
        assert_eq!(&image.as_bytes(0).unwrap()[..4], [30, 20, 10, 255]);
        assert!(decode(&dir.join("missing.png")).is_err());
    }
}
