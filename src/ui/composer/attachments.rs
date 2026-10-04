//! Files attached in the composer and its mode pills (MonoCode `Composer`
//! attachment chips, `ModeCommandPill`): Upload file opens a picker, files
//! dropped from the Finder attach too, Plan mode and Draft show as removable
//! pills beside "+".

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClipboardEntry, Context, ImageFormat, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, SharedString, Styled, div, prelude::*, px,
};

use super::mode_commands::{self, ModeCommand};
use crate::app::{BenCodeApp, now_ms};
use crate::harness::attachments;

/// MonoCode `MAX_ATTACHMENTS`: files one turn carries.
const MAX_ATTACHMENTS: usize = 20;

/// MonoCode's paste warnings: nothing could be read, or the turn is full.
fn attach_error(asked: usize, loaded: usize, wanted: usize, fitted: usize) -> Option<String> {
    if asked > 0 && loaded == 0 {
        Some(
            "Nothing to attach from that path — the file may have been moved, renamed, or deleted."
                .to_string(),
        )
    } else if fitted < wanted {
        Some(format!(
            "Attached {fitted} of {wanted} copied files. A turn carries up to {MAX_ATTACHMENTS}."
        ))
    } else {
        None
    }
}

fn image_ext(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        _ => "png",
    }
}

impl BenCodeApp {
    /// "+" › Upload file: pick files for the focused thread.
    pub fn open_attachment_dialog(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(err)) => {
                    log::error!("file picker failed: {err:#}");
                    return;
                }
            };
            if let Err(err) = this.update(cx, |app, cx| app.attach_paths(paths, cx)) {
                log::debug!("files picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Reads `paths` off the UI thread and adds them to the focused thread's
    /// composer (images inline for vision, other files by path).
    pub fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        let stamp = now_ms();
        let paths_len = paths.len();
        let task = cx.background_executor().spawn(async move {
            paths
                .iter()
                .enumerate()
                .filter_map(|(ix, path)| {
                    match attachments::load(path, format!("att-{stamp}-{ix}")) {
                        Ok(file) => Some(file),
                        Err(err) => {
                            log::warn!("could not attach {}: {err}", path.display());
                            None
                        }
                    }
                })
                .collect::<Vec<_>>()
        });
        let asked = paths_len;
        cx.spawn(async move |this, cx| {
            let files = task.await;
            let added = this.update(cx, |app, cx| {
                let list = app.composer_attachments.entry(session_id).or_default();
                let loaded = files.len();
                let fresh: Vec<_> = files
                    .into_iter()
                    .filter(|file| !list.iter().any(|f| f.path == file.path))
                    .collect();
                let room = MAX_ATTACHMENTS.saturating_sub(list.len());
                let (wanted, fitted) = (fresh.len(), fresh.len().min(room));
                list.extend(fresh.into_iter().take(fitted));
                // MonoCode `pasteError`.
                app.attach_error = attach_error(asked, loaded, wanted, fitted);
                cx.notify();
            });
            if let Err(err) = added {
                log::debug!("attachments loaded after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `onPaste`: images and files on the clipboard attach instead
    /// of pasting as text. Returns whether anything was attached.
    pub fn paste_into_composer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        let mut images: Vec<(Vec<u8>, &'static str)> = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    images.push((image.bytes.clone(), image_ext(image.format)))
                }
                ClipboardEntry::ExternalPaths(external) => {
                    paths.extend(external.paths().iter().cloned())
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if paths.is_empty() && images.is_empty() {
            return false;
        }
        let stamp = now_ms();
        let task = cx.background_executor().spawn(async move {
            let dir = std::env::temp_dir().join("bencode-paste");
            if let Err(err) = std::fs::create_dir_all(&dir) {
                log::error!("could not save pasted image: {err}");
                return paths;
            }
            for (ix, (bytes, ext)) in images.into_iter().enumerate() {
                let file = dir.join(format!("pasted-{stamp}-{ix}.{ext}"));
                match std::fs::write(&file, bytes) {
                    Ok(()) => paths.push(file),
                    Err(err) => log::error!("could not save pasted image: {err}"),
                }
            }
            paths
        });
        cx.spawn(async move |this, cx| {
            let paths = task.await;
            if let Err(err) = this.update(cx, |app, cx| app.attach_paths(paths, cx)) {
                log::debug!("paste after app drop: {err:#}");
            }
        })
        .detach();
        true
    }

    fn remove_attachment(&mut self, session_id: &str, id: &str, cx: &mut Context<Self>) {
        if let Some(list) = self.composer_attachments.get_mut(session_id) {
            list.retain(|f| f.id != id);
        }
        cx.notify();
    }

    /// MonoCode `AttachmentChip`s above the prompt (`flex-wrap gap-1.5
    /// px-3 pt-2`): images as 36px thumbnails with a round remove badge,
    /// other files as a small chip with their icon.
    pub(super) fn render_attachment_chips(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session_id = self.selected_session_id.clone()?;
        let files = self.composer_attachments.get(&session_id)?;
        if files.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let chips = files.iter().map(|file| {
            let (sid, id) = (session_id.clone(), file.id.clone());
            let remove = cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.remove_attachment(&sid, &id, cx)
            });
            if file.is_image() {
                let hover = colors.fg.opacity(0.3);
                return div()
                    .id(SharedString::from(format!("att-{}", file.id)))
                    .relative()
                    .size(px(36.0))
                    .flex_none()
                    .tooltip(Tooltip::text(file.path.clone()))
                    .child(
                        div().size_full().rounded(px(8.0)).overflow_hidden().child(
                            gpui::img(std::path::PathBuf::from(&file.path))
                                .size_full()
                                .object_fit(gpui::ObjectFit::Cover),
                        ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("att-remove-{}", file.id)))
                            .absolute()
                            .top(px(-4.0))
                            .right(px(-4.0))
                            .size(px(20.0))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(colors.fg.opacity(0.2))
                            .shadow_sm()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .tooltip(Tooltip::text("Remove"))
                            .on_click(remove)
                            .child(Icon::new(IconName::X).size(IconSize::Xs).color(colors.fg)),
                    )
                    .into_any_element();
            }
            let hover = colors.fg.opacity(0.15);
            let (icon, tint) = crate::ui::file_tree::resolve_entry_icon(
                &file.name,
                file.mime_type == "inode/directory",
                false,
            );
            div()
                .id(SharedString::from(format!("att-{}", file.id)))
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .py_0p5()
                .px_1()
                .rounded(px(6.0))
                .bg(colors.fg.opacity(0.10))
                .tooltip(Tooltip::text(file.path.clone()))
                .child(Icon::new(icon).size(IconSize::Sm).color(tint))
                .child(
                    div()
                        .max_w(px(140.0))
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(colors.fg.opacity(0.8))
                        .child(file.name.clone()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("att-remove-{}", file.id)))
                        .size(px(16.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .tooltip(Tooltip::text("Remove"))
                        .on_click(remove)
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Xs)
                                .color(colors.fg.opacity(0.4)),
                        ),
                )
                .into_any_element()
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .px_3()
                .pt_2()
                .children(chips)
                .into_any_element(),
        )
    }

    /// MonoCode `ModeCommandPill`s beside "+": Plan in the plan colour,
    /// Draft dashed. A mode shows when chosen from "+" or typed as a
    /// leading `/plan` / `/draft`; clicking the pill turns it off.
    pub(super) fn render_mode_pills(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(session_id) = self.selected_session_id.clone() else {
            return Vec::new();
        };
        let colors = &cx.theme().colors;
        let typed = mode_commands::leading_mode(self.prompt_input.read(cx).text()).map(|(m, _)| m);
        let on = |mode: ModeCommand| {
            typed == Some(mode)
                || match mode {
                    ModeCommand::Plan => self.plan_mode.contains(&session_id),
                    ModeCommand::Draft => self.draft_mode.contains(&session_id),
                }
        };
        [ModeCommand::Plan, ModeCommand::Draft]
            .into_iter()
            .filter(|m| on(*m))
            .map(|mode| {
                let (label, icon, title) = match mode {
                    ModeCommand::Plan => ("Plan", IconName::Lightbulb, "Turn off Plan mode"),
                    ModeCommand::Draft => ("Draft", IconName::CircleDashed, "Turn off Draft mode"),
                };
                let pill = div()
                    .id(SharedString::from(format!("mode-pill-{label}")))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .h(px(26.0))
                    .px_1p5()
                    .rounded(px(6.0))
                    .text_size(px(11.0))
                    .cursor_pointer()
                    .tooltip(Tooltip::text(title))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, false, cx)));
                let pill = match mode {
                    ModeCommand::Plan => {
                        let (fill, hover) =
                            (colors.warning.opacity(0.12), colors.warning.opacity(0.18));
                        pill.bg(fill)
                            .hover(move |s| s.bg(hover))
                            .text_color(colors.warning.opacity(0.9))
                    }
                    ModeCommand::Draft => {
                        let hover = colors.fg.opacity(0.10);
                        pill.border_1()
                            .border_dashed()
                            .border_color(colors.fg.opacity(0.25))
                            .bg(colors.fg.opacity(0.05))
                            .hover(move |s| s.bg(hover))
                            .text_color(colors.fg.opacity(0.7))
                    }
                };
                let ink = match mode {
                    ModeCommand::Plan => colors.warning.opacity(0.9),
                    ModeCommand::Draft => colors.fg.opacity(0.7),
                };
                pill.child(Icon::new(icon).size(IconSize::Xs).color(ink))
                    .child(label)
                    .child(Icon::new(IconName::X).size(IconSize::Xs).color(ink))
                    .into_any_element()
            })
            .collect()
    }

    /// "+" › Plan mode / Draft: flips that mode.
    pub(super) fn toggle_mode(&mut self, draft: bool, cx: &mut Context<Self>) {
        let mode = if draft {
            ModeCommand::Draft
        } else {
            ModeCommand::Plan
        };
        let Some(sid) = self.selected_session_id.clone() else {
            return;
        };
        let typed = mode_commands::leading_mode(self.prompt_input.read(cx).text()).map(|(m, _)| m);
        let on = typed == Some(mode)
            || match mode {
                ModeCommand::Plan => self.plan_mode.contains(&sid),
                ModeCommand::Draft => self.draft_mode.contains(&sid),
            };
        self.set_mode(mode, !on, cx);
    }

    /// MonoCode: the modes exclude each other, and turning one off (or
    /// another on) also takes its leading `/command` out of the prompt.
    fn set_mode(&mut self, mode: ModeCommand, on: bool, cx: &mut Context<Self>) {
        let Some(sid) = self.selected_session_id.clone() else {
            return;
        };
        let text = self.prompt_input.read(cx).text().to_string();
        if let Some((typed, _)) = mode_commands::leading_mode(&text)
            && (!on || typed != mode)
        {
            let (_, rest) = mode_commands::strip_leading_mode(&text);
            self.prompt_input
                .update(cx, |input, cx| input.set_text(rest, cx));
        }
        self.plan_mode.remove(&sid);
        self.draft_mode.remove(&sid);
        if on {
            match mode {
                ModeCommand::Plan => self.plan_mode.insert(sid),
                ModeCommand::Draft => self.draft_mode.insert(sid),
            };
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_warnings() {
        assert!(
            attach_error(2, 0, 0, 0)
                .unwrap()
                .starts_with("Nothing to attach")
        );
        assert_eq!(
            attach_error(25, 25, 25, 20).as_deref(),
            Some("Attached 20 of 25 copied files. A turn carries up to 20.")
        );
        assert_eq!(attach_error(2, 2, 2, 2), None);
    }
}
