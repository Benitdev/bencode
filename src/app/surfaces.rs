//! The full-height views that replace the workspace to the right of the
//! rail: Search, Inbox, Notes, Automations and Settings. Only one is open
//! at a time; Settings returns to the view it was opened from (MonoCode
//! `App.tsx:9788-9950,10881-11160`).

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::window_drag::claim_press;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Search,
    Inbox,
    Notes,
    Automations,
    Settings,
}

impl Surface {
    fn title(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Inbox => "Inbox",
            Self::Notes => "Notes",
            Self::Automations => "Automations",
            Self::Settings => "Settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Search => IconName::Search,
            Self::Inbox => IconName::Inbox,
            Self::Notes => IconName::NotebookPen,
            Self::Automations => IconName::Zap,
            Self::Settings => IconName::Settings,
        }
    }
}

impl BenCodeApp {
    pub fn surface_open(&self, surface: Surface) -> bool {
        self.surface == Some(surface)
    }

    /// Shows `surface` in place of the workspace, closing any other view.
    pub(crate) fn show_surface(&mut self, surface: Surface, cx: &mut Context<Self>) {
        if self.surface == Some(surface) {
            return;
        }
        if self.surface == Some(Surface::Notes) {
            self.save_note_if_dirty(cx);
        }
        if surface == Surface::Settings {
            self.settings_return = self.surface;
            match self.settings_tab {
                crate::ui::settings_modal::SettingsTab::Worktrees => self.open_worktrees_page(cx),
                crate::ui::settings_modal::SettingsTab::Providers => {
                    self.load_accounts_page(false, cx)
                }
                _ => {}
            }
        }
        self.surface = Some(surface);
        cx.notify();
    }

    /// Closes the open view; Settings goes back to where it came from.
    pub fn close_surface(&mut self, cx: &mut Context<Self>) {
        match self.surface {
            Some(Surface::Notes) => {
                self.save_note_if_dirty(cx);
                self.notes.pending_delete = None;
            }
            Some(Surface::Automations) => self.automations.pending_delete = None,
            Some(Surface::Settings) => {
                self.surface = self.settings_return.take();
                cx.notify();
                return;
            }
            _ => {}
        }
        self.surface = None;
        cx.notify();
    }

    /// The open view with its 40px header, or `None` for the workspace.
    pub(crate) fn render_surface(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let surface = self.surface?;
        let body = match surface {
            Surface::Search => self.render_search_body(cx),
            Surface::Inbox => self.render_inbox_body(cx),
            Surface::Notes => self.render_notes_body(cx),
            Surface::Automations => self.render_automations_body(cx),
            Surface::Settings => self.render_settings_body(cx),
        };
        // In MonoCode's `body-glass` column, like the workspace.
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(glass.body(colors.bg))
                .child(self.render_surface_header(surface, cx))
                .child(div().flex_1().min_h_0().overflow_hidden().child(body))
                .into_any_element(),
        )
    }

    /// MonoCode's view header: icon and title, with no close button. Beside
    /// the rail its Back / Forward leave the view; with the rail closed the
    /// header leads with `OverlayNav` (Back, Toggle Sidebar).
    fn render_surface_header(&self, surface: Surface, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let compact = self.compact_rail_active();
        let beside_rail = self.is_rail_open || compact;
        self.window_drag_region(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(crate::ui::scale::px(crate::ui::sidebar::TITLEBAR_HEIGHT)) // MonoCode `h-10`
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            // Beside the icon rail only what the traffic lights overhang
            // it by (MonoCode `compactRail`: `w-4`).
            .when(!self.is_rail_open && cfg!(target_os = "macos"), |el| {
                let gap = if compact { 16.0 } else { 72.0 };
                el.child(div().flex_none().w(gpui::px(gap)))
            })
            .when(!beside_rail, |el| {
                el.child(
                    claim_press(div())
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(
                            IconButton::new("surface-back", IconName::ChevronLeft)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Back (⌘[)")
                                .on_click(cx.listener(|this, _, _, cx| this.close_surface(cx))),
                        )
                        .child(
                            IconButton::new("surface-toggle-projects", IconName::PanelLeft)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Toggle Sidebar (⌘B)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.is_rail_open = true;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                Icon::new(surface.icon())
                    .size(IconSize::Sm)
                    .color(colors.fg_muted),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.fg)
                    .map(|el| {
                        if surface != Surface::Settings {
                            return el.child(surface.title());
                        }
                        // MonoCode's `Settings / <page>` crumb.
                        el.child(
                            div()
                                .text_color(colors.fg.opacity(0.45))
                                .child(surface.title()),
                        )
                        .child(div().text_color(colors.fg.opacity(0.25)).child("/"))
                        .child(div().truncate().child(self.settings_tab.label_icon().0))
                    }),
            )
            .when(
                surface == Surface::Settings
                    && self.settings_tab == crate::ui::settings_modal::SettingsTab::Appearance,
                |el| el.child(claim_press(div()).child(self.render_restore_appearance(cx))),
            )
    }
}
