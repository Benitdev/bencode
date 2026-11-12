//! Over the composer of a thread whose model was switched to another agent
//! (`app/transfer.rs`): the next message goes there with a recap of the
//! conversation, said before it is sent rather than only after.

use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::app::transfer::pending_transfer;
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Text, Tone as _};

impl BenCodeApp {
    pub(super) fn render_transfer_notice(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if self.is_bot_session(&session.id) || self.is_agent_running_in(&session.id) {
            return None;
        }
        let transfer = pending_transfer(session)?;
        let to = HarnessKind::from_id(&session.harness)?;
        let from = match transfer.model.trim() {
            "" => transfer.from.label().to_string(),
            model => format!("{} ({model})", transfer.from.label()),
        };
        let fg = cx.theme().colors.fg;
        Some(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .px_3()
                .pb_1p5()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .child(HarnessIcon::new(to.id()).size(px(12.0)))
                .child(format!(
                    "{} continues this thread with a recap of the conversation with {from}.",
                    to.label()
                ))
                .into_any_element(),
        )
    }
}
