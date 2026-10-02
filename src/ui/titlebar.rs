//! Top bar: workspace tabs (Ely `TabBar`: select, close, add, reorder), a
//! drop zone that detaches a dragged pane into its own tab, and split.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::shell::{TabBar, WindowTab};
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    WindowControlArea, div,
};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::drag_drop::DraggedPane;
use crate::ui::layout::{SplitDir, WorkspaceTab, leaf_count};

const NEW_TAB_TITLE: &str = "New session";
const UNTITLED: &str = "Untitled thread";

impl BenCodeApp {
    /// A tab is labelled by its focused thread, plus how many other panes it holds.
    fn window_tab(&self, tab: &WorkspaceTab) -> WindowTab {
        let title = self
            .sessions
            .iter()
            .find(|s| s.id == tab.focused)
            .map_or(NEW_TAB_TITLE, |s| {
                if s.title.trim().is_empty() {
                    UNTITLED
                } else {
                    s.title.as_str()
                }
            });
        let panes = leaf_count(&tab.layout);
        if panes > 1 {
            WindowTab::new(tab.id.clone(), format!("{title} +{}", panes - 1))
                .icon(IconName::Columns2)
        } else {
            WindowTab::new(tab.id.clone(), title.to_string()).icon(IconName::MessageSquare)
        }
    }

    fn render_tab_bar(&self, cx: &Context<Self>) -> TabBar {
        let weak = cx.entity().downgrade();
        let (select, close) = (weak.clone(), weak.clone());
        let mut bar = self
            .deck_tabs()
            .into_iter()
            .fold(TabBar::new("titlebar-tabs"), |bar, tab| {
                bar.tab(self.window_tab(tab))
            });
        if let Some(active) = self.tabs.active_id() {
            bar = bar.selected(active.to_string());
        }
        bar.on_select(move |id: &SharedString, _, cx| {
            if let Err(err) = select.update(cx, |this, cx| this.switch_tab(id, cx)) {
                log::debug!("tab select after app drop: {err:#}");
            }
        })
        .on_close(move |id: &SharedString, _, cx| {
            if let Err(err) = close.update(cx, |this, cx| this.close_tab(id, cx)) {
                log::debug!("tab close after app drop: {err:#}");
            }
        })
        .on_reorder(move |from, to, _, cx| {
            if let Err(err) = weak.update(cx, |this, cx| this.reorder_open_tabs(from, to, cx)) {
                log::debug!("tab reorder after app drop: {err:#}");
            }
        })
        .on_add(app_callback(cx, |this, cx| this.create_new_session(cx)))
    }

    pub fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let drop_wash = colors.hover;
        div()
            .window_control_area(WindowControlArea::Drag)
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(theme.titlebar_height())
            .w_full()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.bg)
            .px_2()
            .child(
                div()
                    .id("titlebar-tab-drop")
                    .flex()
                    .flex_1()
                    .items_center()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(theme.radius(ely_gpui_component::theme::Radius::Md))
                    .drag_over::<DraggedPane>(move |style, _, _, _| style.bg(drop_wash))
                    .on_drop(cx.listener(|this, dragged: &DraggedPane, _, cx| {
                        this.detach_pane_to_new_tab(&dragged.session_id, cx);
                    }))
                    .child(self.render_tab_bar(cx)),
            )
            .child(
                IconButton::new("titlebar-split-pane", IconName::Columns2)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Split right (⌘D)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.split_active_pane(SplitDir::Right, cx);
                    })),
            )
    }
}
