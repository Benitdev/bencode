//! MonoCode `MonoComposer` in its inline layout (no `controls`), as the
//! floating chat has it: attach, the field and Send on one line in a box
//! of `bg-content/3`, the field moving above the two buttons once it runs
//! past a line or files wait over it. Files dropped on the window attach
//! here ("Drop files to attach"); Enter sends, ⇧Enter is a new line. No
//! emoji button: MonoCode's floating composer has none.
//!
//! Above it, what the turn waits on that is not answered in the
//! transcript: the agent's question (`QuestionForm`) or the provider's
//! usage limit.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Context, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement, Styled, TextRun, Window, div, prelude::*,
};

use super::{CARD_GAP, FailedSend, MonoFloat, RAIL_WIDTH};
use crate::ui::attachment_chip::ChipFile;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `FIELD_CLASSES`: `py-1 text-[13px] leading-4.5`.
const FIELD_SIZE: f32 = 13.0;
const FIELD_LINE: f32 = 18.0;
/// `size-6.5`: attach and Send.
const BUTTON: f32 = 26.0;
/// The gap between the field and each button (`gap-x-1.5`).
const GAP: f32 = 6.0;
/// What sits round the field across the window: the conversation's `px-1`,
/// the form's `p-1.5`, the box's border and the line's `p-1`.
const INSET: f32 = 4.0 + 6.0 + 1.0 + 4.0;

impl MonoFloat {
    /// MonoCode `QuestionForm` over the composer (`max-h-[45%]
    /// overflow-y-auto px-2 pb-2`); the form is the app's, its "Other"
    /// field this window's.
    pub(super) fn render_question(
        &self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.shown.question.as_ref()?;
        let other = self.question_input.clone().into_any_element();
        let sid = session_id.to_string();
        let form = self
            .app
            .update(cx, |app, cx| app.question_form(&sid, other, None, true, cx))?;
        Some(
            div()
                .id("mono-float-question")
                .flex_none()
                .max_h(gpui::relative(0.45))
                .overflow_y_scroll()
                .px_2()
                .pb_2()
                .child(form)
                .into_any_element(),
        )
    }

    /// MonoCode's usage-limit line: `px-3 pb-2 text-[12px] text-content/60`
    /// with its two `text-accent hover:underline` actions.
    pub(super) fn render_usage_limit(
        &self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.shown.usage_limited {
            return None;
        }
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let link = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .text_color(accent)
                .cursor_pointer()
                .hover(|s| s.underline())
                .child(label)
        };
        let sid = session_id.to_string();
        Some(
            div()
                .flex_none()
                .px_3()
                .pb_2()
                .text_size(Text::Base.px())
                .text_color(fg.ink(Ink::Secondary))
                .child("The provider’s usage limit was reached.")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .mt_1()
                        .child(
                            link("mono-float-limit-retry", "Try again").on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.app
                                        .update(cx, |app, cx| app.resume_after_limit(&sid, cx));
                                },
                            )),
                        )
                        .child(div().mx_2().text_color(fg.ink(Ink::Faint)).child("·"))
                        .child(
                            link("mono-float-limit-model", "Change model in BenCode")
                                .on_click(cx.listener(|this, _, _, cx| this.reveal(cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The composer: `shrink-0 px-1 pb-1`, the form's `p-1.5 pt-0`, and the
    /// `rounded-lg border bg-content/3` box (its border at 10%, 20% while
    /// the field has the keyboard, the accent while files are held over).
    pub(super) fn render_composer(
        &self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, accent, danger) = (colors.fg, colors.accent, colors.danger);
        let (files, reading) = self.shown.attached.clone();
        let enabled = !self.shown.removed;
        let chips = self.chips(&files, Some(session_id), cx);
        let typed = self.input.read(cx).text().to_string();
        let ready = enabled && !reading && (!typed.trim().is_empty() || !files.is_empty());
        let handle = self.input.read(cx).focus_handle(cx);
        let focused = handle.is_focused(window);
        // `stacked`: attachments sit over the field like a line of text, so
        // a field of more than a line moves above the buttons for them too.
        let stacked = !files.is_empty() || self.runs_past_a_line(&typed, window);
        let border = if self.file_drag {
            accent.opacity(0.6)
        } else if focused {
            fg.opacity(0.2)
        } else {
            fg.fill(Fill::Strong)
        };
        let field = div()
            .min_w_0()
            .py_1()
            .text_size(px(FIELD_SIZE))
            .line_height(px(FIELD_LINE))
            .child(self.input.clone());
        let attach = self.attach_button(enabled, cx);
        let send = self.send_button(ready, reading, cx);
        let line = if stacked {
            div().p_1().child(field.px(px(6.0))).child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .mt_1()
                    .child(attach)
                    .child(send),
            )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(GAP))
                .min_h(px(36.0))
                .p_1()
                .child(attach)
                .child(field.flex_1())
                .child(send)
        };
        div().flex_none().px_1().pb_1().child(
            div().px(px(6.0)).pb(px(6.0)).child(
                div()
                    .id("mono-float-composer")
                    .relative()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(border)
                    .bg(fg.fill(Fill::Faint))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        window.focus(&handle, cx)
                    })
                    .children(self.shown.attach_error.clone().map(|error| {
                        // `px-3 pt-2 text-xs text-red-400`
                        div()
                            .px_3()
                            .pt_2()
                            .text_size(Text::Base.px())
                            .text_color(danger)
                            .child(error)
                    }))
                    .when(!chips.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1p5()
                                .px(px(10.0))
                                .pt_2()
                                .children(chips),
                        )
                    })
                    .child(line)
                    .when(self.file_drag, |el| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(8.0))
                                .bg(accent.opacity(0.08))
                                .text_size(Text::Base.px())
                                .text_color(fg.ink(Ink::Body))
                                .child("Drop files to attach"),
                        )
                    }),
            ),
        )
    }

    /// Whether `text` takes more than a line in the field at its inline
    /// width (MonoCode measures there, so a field moved above the buttons
    /// does not come back down for being wider).
    fn runs_past_a_line(&self, text: &str, window: &Window) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.contains('\n') {
            return true;
        }
        let card = window.bounds().size.width - px(RAIL_WIDTH + CARD_GAP);
        let inline: Pixels = card - px(INSET * 2.0 + BUTTON * 2.0 + GAP * 2.0);
        let run = TextRun {
            len: text.len(),
            font: window.text_style().font(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = window
            .text_system()
            .shape_line(
                SharedString::from(text.to_string()),
                px(FIELD_SIZE),
                &[run],
                None,
            )
            .width;
        width > inline
    }

    /// `size-6.5 rounded-md bg-content/8 text-content/55
    /// hover:bg-content/12 hover:text-content`, a `+` at `size-3.5`.
    fn attach_button(&self, enabled: bool, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        div()
            .id("mono-float-attach")
            .group("mono-float-attach")
            .flex()
            .flex_none()
            .size(px(BUTTON))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(fg.fill(Fill::Selected))
            .tooltip(Tooltip::text("Attach files"))
            .map(|el| {
                if enabled {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(fg.fill(Fill::Strong)))
                        .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx)))
                } else {
                    el.opacity(0.5)
                }
            })
            .child(
                Icon::new(IconName::Plus)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Secondary))
                    .group_hover_color("mono-float-attach", fg),
            )
            .into_any_element()
    }

    /// `primary-action size-6.5 rounded-md`, an arrow at `size-3.5`.
    fn send_button(&self, ready: bool, reading: bool, cx: &Context<Self>) -> AnyElement {
        let (bg, ink, hover) = crate::ui::appearance::primary_action(ready, cx);
        let tip = if reading {
            "Reading attachments…"
        } else {
            "Send (Enter)"
        };
        div()
            .id("mono-float-send")
            .flex()
            .flex_none()
            .size(px(BUTTON))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(bg)
            .tooltip(Tooltip::text(tip))
            .when(ready, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
            })
            .child(Icon::new(IconName::ArrowUp).size(IconSize::Sm).color(ink))
            .into_any_element()
    }

    /// Enter or Send: the message goes to the Mono, or, when the app cannot
    /// take it, stays in the chat as "Not sent" with its files.
    pub(super) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.shown.removed {
            return;
        }
        let text = self.input.read(cx).text().trim().to_string();
        let Some(session_id) = self.session_id(cx) else {
            return;
        };
        let (files, reading) = self.attached(&session_id, cx);
        // Files still being read go with the next Enter, not half of them now.
        if reading || (text.is_empty() && files.is_empty()) {
            return;
        }
        let sent = self.app.update(cx, |app, cx| {
            app.send_mono_message(&session_id, text.clone(), cx)
        });
        match sent {
            // One that could not be sent waits for its Retry: this one
            // stays in the composer, its files with it, rather than take
            // its place.
            Err(error) if self.failed.is_some() => {
                log::warn!("mono message not sent while another waits for Retry: {error}");
            }
            Err(error) => {
                self.input.update(cx, |input, cx| input.set_text("", cx));
                self.own_attach = false;
                let files = self.take_attachments(&session_id, cx);
                self.failed = Some(FailedSend {
                    session_id,
                    text,
                    files,
                    error,
                });
            }
            // A message sent does not drop the one still waiting for Retry.
            Ok(()) => {
                self.input.update(cx, |input, cx| input.set_text("", cx));
                self.own_attach = false;
            }
        }
        self.relist(cx);
        cx.notify();
    }

    /// Retry on a message that could not be sent: it goes with its own
    /// files, and whatever waits in the composer stays there.
    pub(super) fn retry(&mut self, cx: &mut Context<Self>) {
        let Some(failed) = self.failed.take() else {
            return;
        };
        let session_id = failed.session_id.clone();
        let waiting = self.take_attachments(&session_id, cx);
        let sent = self.app.update(cx, |app, cx| {
            app.thread_mut(&session_id).attachments = failed.files;
            app.send_mono_message(&session_id, failed.text.clone(), cx)
        });
        let files = match sent {
            Ok(()) => Vec::new(),
            Err(_) => self.take_attachments(&session_id, cx),
        };
        if let Err(error) = sent {
            self.failed = Some(FailedSend {
                session_id: session_id.clone(),
                text: failed.text,
                files,
                error,
            });
        }
        self.app.update(cx, |app, cx| {
            app.thread_mut(&session_id).attachments.extend(waiting);
            cx.notify();
        });
        self.relist(cx);
        cx.notify();
    }

    fn take_attachments(
        &self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Vec<crate::harness::Attachment> {
        self.app.update(cx, |app, _| {
            std::mem::take(&mut app.thread_mut(session_id).attachments)
        })
    }

    /// The files waiting in the Mono's composer, and whether more are
    /// still being read. They are the thread's own, so the chat in
    /// BenCode's window shows the same ones.
    pub(super) fn attached(&self, session_id: &str, cx: &App) -> (Vec<ChipFile>, bool) {
        self.app
            .read(cx)
            .thread(session_id)
            .map_or((Vec::new(), false), |thread| {
                (
                    thread.attachments.iter().map(ChipFile::from).collect(),
                    thread.attaching > 0,
                )
            })
    }

    /// ⌘V with images or files on the clipboard; false for plain text,
    /// which the field pastes itself.
    pub(super) fn paste_files(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session_id) = self.session_id(cx).filter(|_| !self.shown.removed) else {
            return false;
        };
        let attached = self
            .app
            .update(cx, |app, cx| app.paste_files_into(session_id, cx));
        self.own_attach |= attached;
        attached
    }

    /// Files dropped on the window, from the Finder.
    pub(super) fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id(cx).filter(|_| !self.shown.removed) else {
            return;
        };
        self.own_attach = true;
        self.app
            .update(cx, |app, cx| app.attach_paths_to(session_id, paths, cx));
    }

    /// MonoCode `MonoComposer`'s +: pick files for the next message.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id(cx) else {
            return;
        };
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
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
            let attached = this.update(cx, |this, cx| {
                this.own_attach = true;
                this.app
                    .update(cx, |app, cx| app.attach_paths_to(session_id, paths, cx));
            });
            if let Err(err) = attached {
                log::debug!("files picked after the floating chat closed: {err:#}");
            }
        })
        .detach();
    }
}
