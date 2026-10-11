//! MonoCode `GitChangesPanel`'s `fail`: a failed git action in a
//! `window.alert`. BenCode's dialog names the action, reads git's output
//! as a sentence (no `error:` / `fatal:` prefixes), sets git's hints apart,
//! keeps long output in a scrolling box, and copies the raw text.

use super::*;
use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::theme::TextSize;

/// A failed git action, shown in a dialog until it is dismissed.
#[derive(Clone, Debug)]
pub struct GitError {
    /// What failed, as the dialog's title ("Couldn't pull").
    pub title: &'static str,
    /// git's output (or the error text) as it came.
    pub output: String,
    /// Copy was pressed; the button says so.
    pub copied: bool,
}

impl GitError {
    pub fn new(title: &'static str, output: impl Into<String>) -> Self {
        Self {
            title,
            output: output.into(),
            copied: false,
        }
    }

    /// The title for a `run_changes_action` failure.
    pub fn for_busy(busy: &Busy, output: impl Into<String>) -> Self {
        let title = match busy {
            Busy::Pull => "Couldn't pull",
            Busy::Commit => "Couldn't commit",
            Busy::Pr => "Couldn't push or open the pull request",
            Busy::Sync => "Couldn't sync",
            Busy::Generate => "Couldn't write a commit message",
            Busy::Undo => "Couldn't undo the commit",
            Busy::Revert => "Couldn't revert the commit",
            Busy::All | Busy::File(_) | Busy::Folder(_) => "Couldn't update your changes",
        };
        Self::new(title, output)
    }
}

/// What a git action's work failed with: git's output, and a title of its
/// own when the action's (`GitError::for_busy`) would name the wrong step.
pub struct GitFailure {
    title: Option<&'static str>,
    output: String,
}

impl GitFailure {
    pub fn titled(title: &'static str, output: impl Into<String>) -> Self {
        Self {
            title: Some(title),
            output: output.into(),
        }
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    /// The dialog for this failure of the action run as `busy`.
    pub fn into_error(self, busy: &Busy) -> GitError {
        match self.title {
            Some(title) => GitError::new(title, self.output),
            None => GitError::for_busy(busy, self.output),
        }
    }
}

impl From<String> for GitFailure {
    fn from(output: String) -> Self {
        Self {
            title: None,
            output,
        }
    }
}

/// git's output read for people.
#[derive(Debug, PartialEq, Eq)]
struct Reading {
    /// The first `error:` / `fatal:` line, else the first line.
    summary: String,
    /// The other lines, in order.
    rest: Vec<String>,
    /// git's `hint:` lines, joined into one paragraph.
    hint: Option<String>,
}

/// More lines than this and the raw output shows in a box instead.
const MAX_REST: usize = 2;

fn read(output: &str) -> Reading {
    let mut lines: Vec<(bool, String)> = Vec::new();
    let mut hints: Vec<&str> = Vec::new();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(hint) = line.strip_prefix("hint:") {
            let hint = hint.trim();
            if !hint.is_empty() {
                hints.push(hint);
            }
            continue;
        }
        // A warning before the failure is not what failed.
        let stripped = [("error:", true), ("fatal:", true), ("warning:", false)]
            .iter()
            .find_map(|(prefix, failure)| Some((line.strip_prefix(prefix)?, *failure)));
        // git writes `error: cannot …`; other tools' text is left as it is.
        let text = match stripped {
            Some((rest, _)) => capitalize(rest.trim()),
            None => line.to_string(),
        };
        lines.push((stripped.is_some_and(|(_, failure)| failure), text));
    }
    let lead = lines.iter().position(|(flagged, _)| *flagged).unwrap_or(0);
    let summary = if lines.is_empty() {
        "Git failed without saying why.".to_string()
    } else {
        lines.remove(lead).1
    };
    Reading {
        summary,
        rest: lines.into_iter().map(|(_, text)| text).collect(),
        hint: (!hints.is_empty()).then(|| hints.join(" ")),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl BenCodeApp {
    pub fn render_git_error(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let error = self.workspace.git_error.as_ref()?;
        let reading = read(&error.output);
        let theme = cx.theme();
        let colors = &theme.colors;
        let long = reading.rest.len() > MAX_REST;
        let close = app_callback(cx, |this, cx| {
            this.workspace.git_error = None;
            cx.notify();
        });
        let output = error.output.clone();
        let copy = app_callback(cx, move |this, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(output.clone()));
            if let Some(error) = this.workspace.git_error.as_mut() {
                error.copied = true;
            }
            cx.notify();
        });
        let copied = error.copied;

        let message = div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded_full()
                    .bg(Severity::Danger.subtle(colors))
                    .child(
                        Icon::new(Severity::Danger.icon())
                            .size(IconSize::Md)
                            .color(Severity::Danger.color(colors)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .pt(px(6.0))
                    .text_size(theme.text_size(TextSize::Sm))
                    .child(
                        div()
                            .text_color(colors.fg)
                            .font_weight(FontWeight::MEDIUM)
                            .child(reading.summary),
                    )
                    .when(!long, |col| {
                        col.children(
                            reading
                                .rest
                                .into_iter()
                                .map(|line| div().text_color(colors.fg_muted).child(line)),
                        )
                    }),
            );

        let raw = long.then(|| {
            div()
                .id("git-error-output")
                .max_h(px(180.0))
                .overflow_y_scroll()
                .p_3()
                .rounded(px(8.0))
                .bg(colors.surface)
                .border_1()
                .border_color(colors.border)
                .font_family(theme.mono_family.clone())
                .text_size(theme.text_size(TextSize::Xs))
                .text_color(colors.fg_muted)
                .child(error.output.clone())
        });

        let hint = reading.hint.map(|hint| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(Severity::Info.subtle(colors))
                .child(
                    div().flex_none().pt(px(2.0)).child(
                        Icon::new(IconName::Lightbulb)
                            .size(IconSize::Sm)
                            .color(Severity::Info.color(colors)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(colors.fg_muted)
                        .child(hint),
                )
        });

        Some(
            Dialog::new("git-error", error.title, close)
                .child(message)
                .children(raw)
                .children(hint)
                .action(move |_| {
                    Button::new(
                        "git-error-copy",
                        if copied { "Copied" } else { "Copy output" },
                    )
                    .variant(ButtonVariant::Ghost)
                    .icon(if copied {
                        IconName::Check
                    } else {
                        IconName::Copy
                    })
                    .on_click(move |_, window, cx| copy(window, cx))
                })
                .action(|close| {
                    Button::new("git-error-ok", "OK")
                        .primary()
                        .on_click(move |_, window, cx| close(window, cx))
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_prefixes_and_leads_with_the_error() {
        let reading = read(
            "error: cannot pull with rebase: Your index contains uncommitted changes.\n\
             error: Please commit or stash them.",
        );
        assert_eq!(
            reading.summary,
            "Cannot pull with rebase: Your index contains uncommitted changes."
        );
        assert_eq!(reading.rest, vec!["Please commit or stash them."]);
        assert_eq!(reading.hint, None);
    }

    #[test]
    fn sets_hints_apart() {
        let reading = read(
            "To github.com:me/repo.git\n \
             ! [rejected]        main -> main (fetch first)\n\
             error: failed to push some refs to 'github.com:me/repo.git'\n\
             hint: Updates were rejected because the remote contains work that you do\n\
             hint: not have locally.\n\
             hint:\n",
        );
        assert_eq!(
            reading.summary,
            "Failed to push some refs to 'github.com:me/repo.git'"
        );
        assert_eq!(
            reading.rest,
            vec![
                "To github.com:me/repo.git",
                "! [rejected]        main -> main (fetch first)"
            ]
        );
        assert_eq!(
            reading.hint.as_deref(),
            Some(
                "Updates were rejected because the remote contains work that you do not have locally."
            )
        );
    }

    #[test]
    fn a_warning_before_the_failure_is_not_the_summary() {
        let reading = read(
            "warning: redirecting to https://example.com/me/repo.git/\n\
             fatal: Authentication failed for 'https://example.com/me/repo.git/'\n",
        );
        assert_eq!(
            reading.summary,
            "Authentication failed for 'https://example.com/me/repo.git/'"
        );
        assert_eq!(
            reading.rest,
            vec!["Redirecting to https://example.com/me/repo.git/"]
        );
        // Only a warning: it is still the first line.
        assert_eq!(read("warning: nothing to do").summary, "Nothing to do");
    }

    #[test]
    fn a_failure_keeps_its_own_title_over_the_actions() {
        let pushed = GitFailure::titled("Committed, but couldn't push", "rejected");
        assert_eq!(
            pushed.into_error(&Busy::Commit).title,
            "Committed, but couldn't push"
        );
        let plain = GitFailure::from("nothing to commit".to_string());
        assert_eq!(plain.into_error(&Busy::Commit).title, "Couldn't commit");
    }

    #[test]
    fn plain_text_and_empty_output() {
        assert_eq!(read("gh: not signed in").summary, "gh: not signed in");
        assert_eq!(read("  \n").summary, "Git failed without saying why.");
    }
}
