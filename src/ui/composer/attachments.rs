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
        cx.spawn(async move |this, cx| {
            let files = task.await;
            let added = this.update(cx, |app, cx| {
                let list = app.composer_attachments.entry(session_id).or_default();
                for file in files {
                    if !list.iter().any(|f| f.path == file.path) {
                        list.push(file);
                    }
                }
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

    /// MonoCode's chips row above the prompt: `flex-wrap gap-1.5 px-3 pt-2`.
    pub(super) fn render_attachment_chips(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session_id = self.selected_session_id.clone()?;
        let files = self.composer_attachments.get(&session_id)?;
        if files.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let chips =
            files.iter().map(|file| {
                let (sid, id) = (session_id.clone(), file.id.clone());
                let hover = colors.fg.opacity(0.15);
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(24.0))
                    .pl_1p5()
                    .pr_0p5()
                    .max_w(px(220.0))
                    .rounded(px(6.0))
                    .bg(colors.fg.opacity(0.08))
                    .text_size(px(12.0))
                    .text_color(colors.fg.opacity(0.8))
                    .child(
                        Icon::new(if file.is_image() {
                            IconName::Image
                        } else {
                            IconName::FileText
                        })
                        .size(IconSize::Xs)
                        .color(colors.fg_muted),
                    )
                    .child(div().min_w_0().truncate().child(file.name.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("att-remove-{id}")))
                            .size(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .tooltip(Tooltip::text("Remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_attachment(&sid, &id, cx)
                            }))
                            .child(
                                Icon::new(IconName::X)
                                    .size(IconSize::Xs)
                                    .color(colors.fg_muted),
                            ),
                    )
            });
        Some(
            div()
                .flex()
                .flex_wrap()
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
