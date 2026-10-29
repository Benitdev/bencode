//! MonoCode `LaunchedSessionCard`: a session a Bot started, as a line
//! under its reply like an artifact's card: its state (a spinner while it
//! works, a check once done), its title and the project it runs in. A
//! click opens it.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, div,
};

use crate::app::BenCodeApp;
use crate::app::bot_sessions::{LaunchedState, SpawnedSession};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::tokens::Status;
use crate::ui::tokens::{Fill, Text, Tone as _};
use crate::ui::transcript::selection::OwnsPress;

fn state_icon(state: Option<LaunchedState>, cx: &App) -> AnyElement {
    let fg = cx.theme().colors.fg;
    let icon = |name: IconName, color: gpui::Hsla| {
        Icon::new(name)
            .size(IconSize::Sm)
            .color(color)
            .into_any_element()
    };
    let Some(state) = state else {
        return icon(IconName::MessageSquare, Status::Quiet.color(fg));
    };
    let color = Status::from(state).color(fg);
    match state {
        LaunchedState::Working => crate::ui::spinner::ring(IconSize::Sm, cx),
        LaunchedState::Input => icon(IconName::Clock, color),
        LaunchedState::Draft => icon(IconName::CircleDashed, color),
        LaunchedState::Failed => icon(IconName::CircleAlert, color),
        LaunchedState::Stopped => icon(IconName::CircleX, color),
        LaunchedState::Done => icon(IconName::CircleCheck, color.opacity(0.9)),
    }
}

/// `flex w-fit max-w-[min(24rem,100%)] items-center gap-2.5 rounded-xl
/// border py-1.5 pr-1.5 pl-3`, the project in a pill at its end.
pub fn launched_card(
    id: impl Into<SharedString>,
    launch: &SpawnedSession,
    title: String,
    state: Option<LaunchedState>,
    cx: &App,
) -> Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    let project = launch
        .cwd
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&launch.cwd)
        .to_string();
    let tip = state.map_or_else(|| "Open session".to_string(), |s| s.label().to_string());
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_none()
        .min_w_0()
        .max_w(px(384.0))
        .items_center()
        .gap(px(10.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(fg.fill(Fill::Strong))
        .py(px(6.0))
        .pl_3()
        .pr(px(6.0))
        .cursor_pointer()
        .owns_press()
        .hover(move |s| s.border_color(fg.opacity(0.2)).bg(fg.fill(Fill::Hover)))
        .tooltip(Tooltip::text(tip))
        .child(state_icon(state, cx))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(Text::Md.px())
                .text_color(fg)
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .max_w(px(160.0))
                .min_w_0()
                .items_center()
                .gap_1()
                .rounded_full()
                .border_1()
                .border_color(fg.fill(Fill::Strong))
                .bg(fg.fill(Fill::Hover))
                .px_2()
                .py_0p5()
                .text_size(Text::Base.px())
                .text_color(fg.opacity(0.65))
                .child(HarnessIcon::new(&launch.harness).size(px(14.0)))
                .child(div().min_w_0().truncate().child(project)),
        )
}

impl BenCodeApp {
    /// MonoCode `LaunchedSessionView.title`: the session's title as it is
    /// now (renamed once its first turn names it), else the one it was
    /// started with.
    pub fn launched_title(&self, launch: &SpawnedSession) -> String {
        let title = self
            .sessions
            .iter()
            .find(|s| s.id == launch.session_id)
            .map_or(&launch.title, |s| &s.title);
        crate::app::session_list::display_title(title, &launch.harness)
    }

    /// Its session in the project's tabs, leaving the Bot's view (the
    /// shown session's Open in workspace).
    pub fn open_launched_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.sessions.iter().any(|s| s.id == id) {
            return;
        }
        self.leave_bot_view(cx);
        self.open_session(id.to_string(), cx);
    }
}
