//! MonoCode `LaunchedSessionCard`: a session a Mono started, as a line
//! under its reply like an artifact's card: its state (a spinner while it
//! works, a check once done), its title and the project it runs in. A
//! click opens it.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, div,
};

use crate::app::BenCodeApp;
use crate::app::mono_sessions::{LaunchedState, SpawnedSession};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;

fn state_icon(state: Option<LaunchedState>, cx: &App) -> AnyElement {
    let fg = cx.theme().colors.fg;
    let icon = |name: IconName, color: gpui::Hsla| {
        Icon::new(name).size(IconSize::Sm).color(color).into_any_element()
    };
    match state {
        Some(LaunchedState::Working) => crate::ui::spinner::ring(IconSize::Sm, cx),
        Some(LaunchedState::Input) => icon(IconName::Clock, gpui::rgb(0xfbbf24).into()),
        Some(LaunchedState::Draft) => icon(IconName::CircleDashed, fg.opacity(0.45)),
        Some(LaunchedState::Failed) => icon(IconName::CircleAlert, gpui::rgb(0xf87171).into()),
        Some(LaunchedState::Stopped) => icon(IconName::CircleX, fg.opacity(0.45)),
        Some(LaunchedState::Done) => icon(IconName::CircleCheck, gpui::rgba(0x34d399e6).into()),
        None => icon(IconName::MessageSquare, fg.opacity(0.45)),
    }
}

/// `flex w-fit max-w-[min(24rem,100%)] items-center gap-2.5 rounded-xl
/// border py-1.5 pr-1.5 pl-3`, the project in a pill at its end.
pub fn launched_card(
    id: impl Into<SharedString>,
    launch: &SpawnedSession,
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
    let title = crate::app::session_list::display_title(&launch.title, &launch.harness);
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
        .border_color(fg.opacity(0.1))
        .py(px(6.0))
        .pl_3()
        .pr(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.border_color(fg.opacity(0.2)).bg(fg.opacity(0.05)))
        .tooltip(Tooltip::text(tip))
        .child(state_icon(state, cx))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.0))
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
                .border_color(fg.opacity(0.1))
                .bg(fg.opacity(0.05))
                .px_2()
                .py_0p5()
                .text_size(px(12.0))
                .text_color(fg.opacity(0.65))
                .child(HarnessIcon::new(&launch.harness).size(px(14.0)))
                .child(div().min_w_0().truncate().child(project)),
        )
}

impl BenCodeApp {
    /// A launch card opens its session in the project's tabs, leaving the
    /// Mono's view.
    pub fn open_launched_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.sessions.iter().any(|s| s.id == id) {
            return;
        }
        self.leave_mono_view(cx);
        self.open_session(id.to_string(), cx);
    }
}
