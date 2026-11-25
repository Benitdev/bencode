//! The full-height views that replace the workspace to the right of the
//! rail: Inbox, Notes and Automations. Only one is open at a time (MonoCode
//! `App.tsx:9788-9950,10881-11160`).

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::window_drag::claim_press;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Inbox,
    Notes,
    Automations,
}

impl Surface {
    fn title(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Notes => "Notes",
            Self::Automations => "Automations",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Inbox => IconName::Inbox,
            Self::Notes => IconName::NotebookPen,
            Self::Automations => IconName::Zap,
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
        if surface == Surface::Inbox {
            self.read_inbox_translations(cx);
        }
        if surface == Surface::Notes
            && self.notes.selected_id.is_none()
            && let Some(id) = self.notes.items.first().map(|note| note.id.clone())
        {
            self.select_note(id, cx);
        }
        self.surface = Some(surface);
        cx.notify();
    }

    /// Closes the open view.
    pub fn close_surface(&mut self, cx: &mut Context<Self>) {
        match self.surface {
            Some(Surface::Notes) => {
                self.save_note_if_dirty(cx);
                self.notes.pending_delete = None;
            }
            Some(Surface::Automations) => {
                self.automations.pending_delete = None;
                self.automations.pending_discard = None;
                self.automations.pause_pick = None;
            }
            _ => {}
        }
        self.surface = None;
        cx.notify();
    }

    /// The open view with its 40px header, or `None` for the workspace.
    pub(crate) fn render_surface(
        &mut self,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let surface = self.surface?;
        let body = match surface {
            Surface::Inbox => self.render_inbox_body(window, cx),
            Surface::Notes => self.render_notes_body(cx),
            Surface::Automations => self.render_automations_body(cx),
        };
        let glass = self.glass(cx);
        let panel_gap = px(crate::ui::tokens::PANEL_GAP);
        let left_inset = self.workspace_left_inset();
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.render_surface_header(surface, cx))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .pl(left_inset)
                        .pr(panel_gap)
                        .pb(panel_gap)
                        .child(
                            glass
                                .card(div().flex().flex_col().flex_1().min_h_0().min_w_0(), cx)
                                .child(body),
                        ),
                )
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
            .h(px(crate::ui::sidebar::TITLEBAR_HEIGHT)) // MonoCode `h-10`
            .px_3()
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
                    .child(surface.title()),
            )
    }
}
