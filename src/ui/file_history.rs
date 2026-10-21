//! A File History tab (GitLens's File History): the commits that changed
//! one file, newest first; a click opens the commit beside it, a double
//! click keeps it.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::Avatar;
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, AvatarSize, ControlSize, IconSize};
use gpui::{
    AnyElement, ClipboardItem, Context, IntoElement, ParentElement, SharedString, Styled, div,
    uniform_list,
};

use crate::app::BenCodeApp;
use crate::app::blame::Load;
use crate::app::file_pane::PaneTab;
use crate::git::blame::FileCommit;
use crate::ui::rows::{ListRow, row_button};
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Text, Tone as _};

impl BenCodeApp {
    pub(crate) fn render_file_history(&self, tab: &PaneTab, cx: &Context<Self>) -> AnyElement {
        let PaneTab::FileHistory { cwd, path } = tab else {
            return div().into_any_element();
        };
        let load = self.blame.histories.get(&tab.key());
        let count = match load {
            Some(Load::Ready((_, commits))) => Some(commits.len()),
            _ => None,
        };
        let body = match load {
            // Commits open from the repository's top.
            Some(Load::Ready((root, commits))) if !commits.is_empty() => {
                self.render_history_list(root, commits, cx)
            }
            Some(Load::Ready(_)) => empty(
                "No commits yet",
                "This file has not been committed.".to_string(),
            ),
            Some(Load::Failed(err)) => empty("Could not read the history", err.clone()),
            Some(Load::Loading) | None => {
                empty("Reading history…", format!("git log --follow -- {path}"))
            }
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_history_header(cwd, path, count, cx))
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn render_history_header(
        &self,
        cwd: &str,
        path: &str,
        count: Option<usize>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let (reload_cwd, reload_path) = (cwd.to_string(), path.to_string());
        // The file as the workspace names it, else in full.
        let full = format!("{cwd}/{path}");
        let workspace = format!("{}/", self.workspace(cx).cwd);
        let shown_path = full.strip_prefix(&workspace).unwrap_or(&full).to_string();
        let counted = count.map(|n| {
            if n == 1 {
                "1 commit".into()
            } else {
                format!("{n} commits")
            }
        });
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .h(theme.control_height(ControlSize::Lg))
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .child(Icon::new(IconName::History).size(IconSize::Xs))
            .child(div().flex_1().min_w_0().truncate().child(shown_path))
            .children(counted.map(|text| div().flex_none().child(text)))
            .child(
                IconButton::new("file-history-reload", IconName::RefreshCw)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Reload history")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_file_history(reload_cwd.clone(), reload_path.clone(), cx);
                    })),
            )
    }

    fn render_history_list(
        &self,
        cwd: &str,
        commits: &[FileCommit],
        cx: &Context<Self>,
    ) -> AnyElement {
        let now = crate::app::now_ms();
        let (cwd, commits) = (cwd.to_string(), commits.to_vec());
        let len = commits.len();
        let rows = cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
            range
                .filter_map(|ix| commits.get(ix))
                .map(|commit| history_row(&cwd, commit, now, cx))
                .collect::<Vec<_>>()
        });
        let scroll = &self.blame.history_scroll;
        crate::ui::scrollbar::framed(
            "file-history-scrollbar",
            scroll,
            uniform_list("file-history-list", len, rows)
                .track_scroll(scroll)
                .size_full()
                .p(px(6.0))
                .pr(px(6.0) + crate::ui::scrollbar::gutter(scroll)),
        )
        .size_full()
        .into_any_element()
    }
}

/// A commit: its author's initials, its subject, then who, when and its
/// short sha.
fn history_row(cwd: &str, commit: &FileCommit, now: i64, cx: &Context<BenCodeApp>) -> AnyElement {
    let when = crate::ui::relative_time::since(commit.time * 1000, now);
    let line = format!("{} · {when} · {}", commit.author, commit.short_sha);
    let tab = PaneTab::Commit {
        cwd: cwd.to_string(),
        sha: commit.sha.clone(),
        short_sha: commit.short_sha.clone(),
        subject: commit.subject.clone(),
    };
    let sha = commit.sha.clone();
    let weak = cx.entity().downgrade();
    ListRow::new(
        format!("file-history-{}", commit.sha),
        commit.subject.clone(),
    )
    .tile(
        Avatar::new(
            SharedString::from(format!("file-history-avatar-{}", commit.sha)),
            commit.author.clone(),
        )
        .size(AvatarSize::Xs),
    )
    .line(line)
    .tooltip(format!("{} — {}", commit.short_sha, commit.subject))
    .on_click(move |event, _, cx| {
        let tab = tab.clone();
        let pin = event.click_count() == 2;
        let opened = weak.update(cx, |this, cx| this.open_pane_tab(tab, pin, cx));
        if let Err(err) = opened {
            log::debug!("file history click after app drop: {err:#}");
        }
    })
    .action(
        row_button(
            format!("file-history-copy-{}", commit.sha),
            IconName::Copy,
            "Copy commit id",
        )
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()))),
    )
    .into_any_element()
}

fn empty(title: &'static str, body: String) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .child(EmptyState::new("file-history-empty", IconName::History, title).body(body))
        .into_any_element()
}
