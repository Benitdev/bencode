//! Files attached in the composer and its mode pills (MonoCode `Composer`
//! attachment chips, `ModeCommandPill`): Upload file opens a picker, files
//! dropped from the Finder attach too, Plan mode and Draft show as removable
//! pills beside "+".

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, PathPromptOptions,
    SharedString, Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, now_ms};
use crate::harness::attachments;

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

    /// Plan mode / Draft pills beside "+", each with an X to turn it off.
    pub(super) fn render_mode_pills(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(session_id) = self.selected_session_id.clone() else {
            return Vec::new();
        };
        let colors = &cx.theme().colors;
        let mut pills = Vec::new();
        let pill = |label: &'static str, icon: IconName, tint, on_remove: fn(&mut Self, &str)| {
            let sid = session_id.clone();
            let hover = colors.fg.opacity(0.15);
            div()
                .id(SharedString::from(format!("mode-pill-{label}")))
                .flex()
                .items_center()
                .gap_1()
                .h(px(26.0))
                .pl_2()
                .pr_1()
                .rounded(px(6.0))
                .bg(colors.fg.opacity(0.08))
                .text_size(px(11.0))
                .text_color(colors.fg.opacity(0.8))
                .child(Icon::new(icon).size(IconSize::Xs).color(tint))
                .child(label)
                .child(
                    div()
                        .id(SharedString::from(format!("mode-pill-x-{label}")))
                        .size(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            on_remove(this, &sid);
                            cx.notify();
                        }))
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Xs)
                                .color(colors.fg_muted),
                        ),
                )
                .into_any_element()
        };
        if self.plan_mode.contains(&session_id) {
            pills.push(pill("Plan", IconName::Map, colors.warning, |this, sid| {
                this.plan_mode.remove(sid);
            }));
        }
        if self.draft_mode.contains(&session_id) {
            pills.push(pill(
                "Draft",
                IconName::CircleDashed,
                colors.fg_muted,
                |this, sid| {
                    this.draft_mode.remove(sid);
                },
            ));
        }
        pills
    }

    /// Toggles Plan mode or Draft for the focused thread.
    pub(super) fn toggle_mode(&mut self, draft: bool, cx: &mut Context<Self>) {
        let Some(sid) = self.selected_session_id.clone() else {
            return;
        };
        let set = if draft {
            &mut self.draft_mode
        } else {
            &mut self.plan_mode
        };
        if !set.remove(&sid) {
            set.insert(sid);
        }
        cx.notify();
    }
}
