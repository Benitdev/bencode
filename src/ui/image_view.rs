//! Pictures in the file pane: a PNG, JPEG, GIF or WebP opens as a tab that
//! shows it fitted, with its size, instead of the editor refusing a binary
//! file. The bytes are read off the UI thread (a file on a machine too) and
//! drawn through `thumbnail_bytes`, so only a scaled copy is kept. A click
//! opens it full-window in the lightbox. (MonoCode's editor has no image
//! view; the WebView would show one in an `<img>`.)

use std::path::{Path, PathBuf};

use bencode_ui::feedback::EmptyState;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize, TextSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    StatefulInteractiveElement, Styled, StyledImage, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::git::MAX_BLOB_BYTES;
use crate::ui::region::Region;
use crate::ui::thumbnail::{ImageBytes, release_bytes, thumbnail_bytes};

/// The largest a tab draws its picture, in logical pixels: wider than any
/// pane, so a screenshot keeps its detail.
const VIEW_EDGE: f32 = 1600.0;

/// Whether `path` names a picture BenCode decodes (the `image` crate's
/// formats in `Cargo.toml`). SVG is text and stays in the editor.
pub fn is_image_path(path: &str) -> bool {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

/// `bytes` as Finder writes a file's size.
pub fn format_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} bytes"),
        _ if size < 10.0 => format!("{size:.1} {}", UNITS[unit]),
        _ => format!("{size:.0} {}", UNITS[unit]),
    }
}

/// A picture's bytes and size in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub bytes: ImageBytes,
    pub width: u32,
    pub height: u32,
}

impl Picture {
    /// `None` when the bytes are no picture this build decodes.
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        let bytes = ImageBytes::new(bytes);
        let (width, height) = bytes.dimensions()?;
        Some(Self {
            bytes,
            width,
            height,
        })
    }

    /// "1440 × 900 · 212 KB".
    pub fn describe(&self) -> String {
        format!(
            "{} × {} · {}",
            self.width,
            self.height,
            format_size(self.bytes.len())
        )
    }
}

pub enum ImageLoad {
    Loading,
    Ready(Picture),
    Failed(String),
}

/// One picture tab's read; a newer read wins over a slower older one.
pub struct ImageTab {
    load: ImageLoad,
    generation: u64,
}

/// Reads the picture at `path`, here or on its machine.
fn read_picture(path: &str) -> Result<Picture, String> {
    let limit = MAX_BLOB_BYTES;
    let bytes = if crate::remote::is_remote(path) {
        let file = crate::remote::files::read(path, limit + 1).map_err(|e| e.to_string())?;
        if file.truncated {
            return Err(too_large(limit));
        }
        file.bytes.0
    } else {
        let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if len > limit {
            return Err(too_large(limit));
        }
        std::fs::read(path).map_err(|e| e.to_string())?
    };
    Picture::new(bytes).ok_or_else(|| "The file is not a picture BenCode can show.".into())
}

fn too_large(limit: u64) -> String {
    format!(
        "The picture is larger than {}.",
        format_size(limit as usize)
    )
}

impl BenCodeApp {
    /// Opens the picture at `path` (full) as a tab, reading it again when
    /// it is already open, in case it changed.
    pub(crate) fn open_image_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = path.to_string_lossy().into_owned();
        self.open_pane_tab(PaneTab::Image { path: path.clone() }, true, cx);
        let tab = self.image_tabs.entry(path.clone()).or_insert(ImageTab {
            load: ImageLoad::Loading,
            generation: 0,
        });
        tab.generation += 1;
        let generation = tab.generation;
        let read = {
            let path = path.clone();
            cx.background_executor()
                .spawn(async move { read_picture(&path) })
        };
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let landed = this.update(cx, |this, cx| {
                let Some(tab) = this.image_tabs.get_mut(&path) else {
                    return;
                };
                if tab.generation != generation {
                    return;
                }
                if let Err(err) = &read {
                    log::warn!("could not show {path}: {err}");
                }
                let load = match read {
                    Ok(picture) => ImageLoad::Ready(picture),
                    Err(err) => ImageLoad::Failed(err),
                };
                let before = std::mem::replace(&mut tab.load, load);
                if let ImageLoad::Ready(old) = before
                    && !matches!(&tab.load, ImageLoad::Ready(new) if new.bytes == old.bytes)
                {
                    release_bytes(&old.bytes, VIEW_EDGE, cx);
                }
                this.redraw_regions(&[Region::FilePane], cx);
            });
            if let Err(err) = landed {
                log::debug!("picture read after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Lets go of the pictures whose tabs closed.
    pub(crate) fn settle_image_tabs(&mut self, cx: &mut Context<Self>) {
        let pane = &self.file_pane;
        let closed: Vec<String> = (self.image_tabs.keys())
            .filter(|path| {
                let key = PaneTab::Image {
                    path: (*path).clone(),
                }
                .key();
                pane.get(&key).is_none()
            })
            .cloned()
            .collect();
        for path in closed {
            if let Some(ImageTab {
                load: ImageLoad::Ready(picture),
                ..
            }) = self.image_tabs.remove(&path)
            {
                release_bytes(&picture.bytes, VIEW_EDGE, cx);
            }
        }
    }

    pub(crate) fn render_image_tab(&self, path: &str, cx: &Context<Self>) -> AnyElement {
        let load = self.image_tabs.get(path).map(|tab| &tab.load);
        let body = match load {
            Some(ImageLoad::Ready(picture)) => self.render_picture(path, picture, cx),
            Some(ImageLoad::Failed(err)) => centered(
                EmptyState::new("image-failed", IconName::ImageOff, "Could not show picture")
                    .body(err.clone()),
            ),
            Some(ImageLoad::Loading) | None => centered(
                EmptyState::new("image-loading", IconName::FileImage, "Opening picture…")
                    .body("Reading the file."),
            ),
        };
        let picture = match load {
            Some(ImageLoad::Ready(picture)) => Some(picture),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_image_toolbar(path, picture, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(body),
            )
            .into_any_element()
    }

    /// The picture's path in the workspace, its size in pixels and bytes.
    fn render_image_toolbar(
        &self,
        path: &str,
        picture: Option<&Picture>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let root = self.workspace(cx).cwd.clone();
        let shown = Path::new(path)
            .strip_prefix(&root)
            .map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .h(theme.control_height(ControlSize::Lg))
            .w_full()
            .min_w_0()
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .text_size(theme.text_size(TextSize::Xs))
            .text_color(colors.fg_muted)
            .child(
                div()
                    .flex_none()
                    .child(Icon::new(IconName::FileImage).size(IconSize::Xs)),
            )
            .child(div().flex_1().min_w_0().truncate().child(shown))
            .children(picture.map(|picture| {
                div()
                    .flex_none()
                    .text_color(colors.fg_subtle)
                    .child(picture.describe())
            }))
    }

    /// The picture fitted to the pane, never drawn larger than it is.
    fn render_picture(&self, path: &str, picture: &Picture, cx: &Context<Self>) -> AnyElement {
        // The lightbox reads files here only.
        let local = !crate::remote::is_remote(path);
        let full = PathBuf::from(path);
        div()
            .flex()
            .flex_1()
            .size_full()
            .min_h_0()
            .items_center()
            .justify_center()
            .p_4()
            .child(
                thumbnail_bytes(&picture.bytes, VIEW_EDGE)
                    .id("image-tab-picture")
                    .max_w_full()
                    .max_h_full()
                    .object_fit(ObjectFit::Contain)
                    .when(local, |el| {
                        el.cursor_pointer()
                            .tooltip(Tooltip::text("Show full window"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_lightbox(full.clone(), cx);
                            }))
                    }),
            )
            .into_any_element()
    }
}

fn centered(state: EmptyState) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(state)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_extension() {
        assert!(is_image_path("shots/02_style-edit_OK.png"));
        assert!(is_image_path("a/B.JPEG"));
        assert!(is_image_path("x.webp") && is_image_path("x.gif"));
        assert!(!is_image_path("logo.svg"));
        assert!(!is_image_path("png"));
        assert!(!is_image_path("notes.md"));
    }

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1_500), "1.5 KB");
        assert_eq!(format_size(212_000), "212 KB");
        assert_eq!(format_size(3_400_000), "3.4 MB");
    }
}
