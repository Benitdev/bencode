//! Full-window image preview (MonoCode `ImageLightbox`): the image fitted
//! on a dark backdrop, a round close button at the top right; Esc, the
//! button or a click on the backdrop closes it. A right click on the image
//! opens a menu: Copy Image, Copy Path, Reveal in Finder.

use std::path::{Path, PathBuf};

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::IconSize;
use gpui::{
    AnyElement, ClipboardItem, Context, Image, ImageFormat, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels, Point, Styled, StyledImage,
    deferred, div, prelude::*, rgba,
};

use crate::app::BenCodeApp;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuStyle, MenuView};
use crate::ui::scale::px;

/// The preview's layer, over menus and dialogs; its own menu goes above.
const LIGHTBOX_LAYER: usize = 4;
const MENU_WIDTH: f32 = 200.0;

/// The preview's right-click menu: where it opened and the row under the
/// keys.
pub struct LightboxMenu {
    position: Point<Pixels>,
    active: usize,
}

fn menu_entries() -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item(MenuAction::new("copy-image", "Copy Image").icon(IconName::Image, false)),
        MenuEntry::Item(MenuAction::new("copy-path", "Copy Path").icon(IconName::Copy, false)),
        MenuEntry::Separator,
        MenuEntry::Item(
            MenuAction::new("reveal", "Reveal in Finder").icon(IconName::FolderOpen, false),
        ),
    ]
}

/// The clipboard's name for a picture file, from its extension.
fn image_format(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        "svg" => ImageFormat::Svg,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        _ => return None,
    })
}

/// The largest the preview may be drawn, in logical pixels: a window's
/// width, so a screenshot keeps its full resolution.
const LIGHTBOX: f32 = 2048.0;

impl BenCodeApp {
    pub fn open_lightbox(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.lightbox_menu = None;
        if let Some(shown) = self.lightbox.replace(path)
            && self.lightbox.as_ref() != Some(&shown)
        {
            crate::ui::thumbnail::release(shown, LIGHTBOX, cx);
        }
        cx.notify();
    }

    /// True when a preview was open. The full-size image is let go.
    pub fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.lightbox.take() else {
            return false;
        };
        self.lightbox_menu = None;
        crate::ui::thumbnail::release(path, LIGHTBOX, cx);
        cx.notify();
        true
    }

    pub fn lightbox_menu_open(&self) -> bool {
        self.lightbox_menu.is_some()
    }

    fn open_lightbox_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.lightbox_menu = Some(LightboxMenu {
            position,
            active: explorer_menu::first_item(&menu_entries()),
        });
        self.focus_composer_menu(cx);
        cx.notify();
    }

    fn close_lightbox_menu(&mut self, cx: &mut Context<Self>) {
        if self.lightbox_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Keys while the preview's menu holds focus.
    pub fn lightbox_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.lightbox_menu.as_mut() else {
            return false;
        };
        let entries = menu_entries();
        match key {
            "down" | "up" => {
                let dir = if key == "down" { 1 } else { -1 };
                menu.active = explorer_menu::step(&entries, menu.active, dir);
            }
            "enter" | "space" => {
                let active = menu.active;
                self.pick_lightbox_menu(active, cx);
            }
            "escape" => self.lightbox_menu = None,
            _ => return false,
        }
        cx.notify();
        true
    }

    fn pick_lightbox_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        let entries = menu_entries();
        let picked = explorer_menu::pick(&entries, ix);
        self.lightbox_menu = None;
        cx.notify();
        let Some(path) = self.lightbox.clone() else {
            return;
        };
        match picked {
            Some("copy-image") => self.copy_image(path, cx),
            Some("copy-path") => {
                cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()))
            }
            Some("reveal") => cx.reveal_path(&path),
            _ => {}
        }
    }

    /// The picture's bytes, read off the UI thread, onto the clipboard; a
    /// file of no picture type the clipboard names goes as its path.
    fn copy_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(format) = image_format(&path) else {
            cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
            return;
        };
        let read = cx.background_executor().spawn({
            let path = path.clone();
            async move { std::fs::read(&path) }
        });
        cx.spawn(async move |_, cx| match read.await {
            Ok(bytes) => {
                let item = ClipboardItem::new_image(&Image::from_bytes(format, bytes));
                cx.update(|cx| cx.write_to_clipboard(item));
            }
            Err(err) => log::warn!("could not read {} to copy it: {err}", path.display()),
        })
        .detach();
    }

    fn render_lightbox_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.lightbox_menu.as_ref()?;
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu_layered(
            MenuView {
                id: "lightbox-menu",
                entries: &menu_entries(),
                active: menu.active,
                place: MenuPlace::At(menu.position),
                width: MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            MenuStyle::Explorer,
            LIGHTBOX_LAYER + 1,
            move |ix, _, cx| {
                let updated = hover_app.update(cx, |this, cx| {
                    if let Some(menu) = this.lightbox_menu.as_mut()
                        && menu.active != ix
                    {
                        menu.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = updated {
                    log::debug!("lightbox menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_lightbox_menu(ix, cx)) {
                    log::debug!("lightbox menu pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                if let Err(err) = close_app.update(cx, |this, cx| this.close_lightbox_menu(cx)) {
                    log::debug!("lightbox menu close after app drop: {err:#}");
                }
            },
            cx,
        ))
    }

    pub fn render_lightbox(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let path = self.lightbox.clone()?;
        let close = div()
            .id("lightbox-close")
            .absolute()
            .top_4()
            .right_4()
            .size(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(rgba(0xffffff26))
            .bg(rgba(0x00000073))
            .shadow_lg()
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0x000000a6)))
            .tooltip(Tooltip::text("Close"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_lightbox(cx);
            }))
            .child(
                Icon::new(IconName::X)
                    .size(IconSize::Sm)
                    .color(rgba(0xffffffcc)),
            );
        Some(
            deferred(
                div()
                    .id("lightbox")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .p_6()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x000000d9))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            // A click away from an open menu only closes it.
                            if this.lightbox_menu.is_some() {
                                this.close_lightbox_menu(cx);
                            } else {
                                this.close_lightbox(cx);
                            }
                        }),
                    )
                    .child(
                        // Only the backdrop closes; the image itself does not.
                        div()
                            .max_w_full()
                            .max_h_full()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.open_lightbox_menu(event.position, cx);
                                }),
                            )
                            .child(
                                crate::ui::thumbnail::thumbnail(path, LIGHTBOX)
                                    .max_w_full()
                                    .max_h_full()
                                    .object_fit(ObjectFit::Contain)
                                    .shadow_2xl(),
                            ),
                    )
                    .child(close)
                    .children(self.render_lightbox_menu(cx)),
            )
            .with_priority(LIGHTBOX_LAYER)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_copy_as_their_own_type() {
        assert_eq!(
            image_format(Path::new("/a/Shot.PNG")),
            Some(ImageFormat::Png)
        );
        assert_eq!(image_format(Path::new("b.jpeg")), Some(ImageFormat::Jpeg));
        assert_eq!(image_format(Path::new("c.heic")), None);
        assert_eq!(image_format(Path::new("noext")), None);
    }
}
