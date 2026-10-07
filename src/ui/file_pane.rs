//! The pane to the right of the chat (MonoCode's editor pane and its
//! `SurfaceTabs`): open files, reviews and commits behind one tab strip.
//! The chat stays beside it and takes the full width once it is empty.

use ely_gpui_component::layout::SplitPane;
use ely_gpui_component::navigation::{EditorTab, EditorTabs};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{AnyElement, Axis, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::ui::diff_viewer::DocFocus;

impl BenCodeApp {
    /// Focuses `tab` or opens it (MonoCode `openEditorTab`, `openChangesTab`,
    /// `openCommitTab`); `pin` keeps it from being a preview.
    pub(crate) fn open_pane_tab(&mut self, tab: PaneTab, pin: bool, cx: &mut Context<Self>) {
        if let PaneTab::Changes { cwd, .. } = &tab {
            self.file_pane.drop_reviews(cwd);
        }
        let focus = match &tab {
            PaneTab::Changes {
                focus: Some(path),
                side,
                ..
            } => Some(DocFocus {
                path: path.clone(),
                side: *side,
            }),
            PaneTab::SessionChanges {
                focus: Some(path), ..
            } => Some(DocFocus {
                path: path.clone(),
                side: None,
            }),
            _ => None,
        };
        let is_diff = tab.is_diff();
        let moved = self
            .file_pane
            .get(&tab.key())
            .is_some_and(|open| !open.same_source(&tab));
        let key = self.file_pane.open(tab, pin);
        self.file_pane_focused = true;
        self.settle_file_pane();
        if is_diff {
            self.ensure_diff_doc(&key, focus, cx);
            if moved {
                self.load_diff_doc(&key, cx);
            }
        }
        cx.notify();
    }

    /// After tabs opened, closed or changed: reviews of closed tabs go, and
    /// the editor follows the active file.
    fn settle_file_pane(&mut self) {
        self.diff_docs.retain(|key, _| self.file_pane.get(key).is_some());
        if let Some(PaneTab::File { path }) = self.file_pane.active() {
            self.editor.requested = Some(path.clone());
            self.editor.files.activate(path);
        }
        if self.file_pane.is_empty() {
            self.file_pane_focused = false;
        }
    }

    fn select_pane_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.file_pane.activate(key) {
            self.file_pane_focused = true;
            self.settle_file_pane();
            cx.notify();
        }
    }

    /// Closes a tab; a file with unsaved edits asks first.
    pub(crate) fn request_close_pane_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        match self.file_pane.get(key).cloned() {
            Some(PaneTab::File { path }) => self.request_close_editor_file(&path, cx),
            Some(_) => self.drop_pane_tab(key, cx),
            None => {}
        }
    }

    /// Closes the active tab (⌘W); false when the pane is empty.
    pub(crate) fn close_active_pane_tab(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.file_pane.active_key().map(str::to_string) else {
            return false;
        };
        self.request_close_pane_tab(&key, cx);
        true
    }

    /// Removes a tab without asking.
    pub(crate) fn drop_pane_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.file_pane.close(key).is_some() {
            self.settle_file_pane();
            cx.notify();
        }
    }

    /// The chat, with the file pane to its right while it has tabs.
    pub(crate) fn render_workspace_split(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let chat = div()
            .size_full()
            .flex()
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.file_pane_focused = false))
            .child(self.render_transcript_panel(cx));
        if self.file_pane.is_empty() && self.editor.notice.is_none() {
            return chat.into_any_element();
        }
        let theme = cx.theme();
        let min = theme.pane_min().to_pixels(theme.base_rem() * crate::ui::scale::ui_scale());
        let weak = cx.entity().downgrade();
        let pane = div()
            .size_full()
            .flex()
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.file_pane_focused = true))
            .child(self.render_file_pane(cx));
        SplitPane::new("workspace-file-split", Axis::Horizontal, min)
            .sizes(&self.file_pane_shares)
            .on_resize(move |shares, _window, cx| {
                let landed = weak.update(cx, |this, _| {
                    if let [chat, pane] = shares {
                        this.file_pane_shares = [*chat, *pane];
                    }
                });
                if let Err(err) = landed {
                    log::debug!("file pane resize after app drop: {err:#}");
                }
            })
            .pane(chat)
            .pane(pane)
            .into_any_element()
    }

    fn render_file_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        let body = match self.file_pane.active() {
            Some(tab) if tab.is_diff() => self.render_diff_doc(&tab.key(), cx),
            _ => self.render_editor_file(cx).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(glass.fill(colors.bg))
            .border_l_1()
            .border_color(colors.border)
            .when(!self.file_pane.is_empty(), |el| el.child(self.render_pane_tabs(cx)))
            .children(self.render_editor_notice(cx))
            .child(div().flex().flex_col().flex_1().min_h_0().overflow_hidden().child(body))
            .children(self.render_editor_close_confirm(cx))
    }

    fn render_pane_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let tabs = self.file_pane.entries().iter().map(|entry| {
            let (label, _) = entry.tab.label();
            let (icon, dirty) = match &entry.tab {
                PaneTab::File { path } => (IconName::FileCode, self.editor.files.is_dirty(path)),
                PaneTab::Commit { .. } => (IconName::GitCommitHorizontal, false),
                PaneTab::Review { .. } | PaneTab::Changes { .. } | PaneTab::SessionChanges { .. } => {
                    (IconName::GitBranch, false)
                }
            };
            EditorTab::new(entry.tab.key(), label)
                .icon(icon)
                .dirty(dirty)
                .preview(entry.preview)
        });
        let strip = EditorTabs::new("file-pane-tabs", tabs)
            .on_select(cx.listener(|this, key: &SharedString, _, cx| this.select_pane_tab(key, cx)))
            .on_close(cx.listener(|this, key: &SharedString, _, cx| {
                this.request_close_pane_tab(key, cx)
            }))
            .on_keep(cx.listener(|this, key: &SharedString, _, cx| {
                this.file_pane.keep(key);
                cx.notify();
            }))
            .on_reorder({
                let weak = cx.entity().downgrade();
                move |from, to, _window, cx| {
                    let moved = weak.update(cx, |this, cx| {
                        this.file_pane.reorder(from, to);
                        cx.notify();
                    });
                    if let Err(err) = moved {
                        log::debug!("tab reorder after app drop: {err:#}");
                    }
                }
            });
        match self.file_pane.active_key() {
            Some(active) => strip.selected(active.to_string()),
            None => strip,
        }
    }
}
