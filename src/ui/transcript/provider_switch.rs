//! MonoCode `HandoffDivider` (GPUI `transcript/provider_switch.rs`): a rule
//! with the new agent's mark over the first message another agent answers,
//! and on a transferred turn what it came from (`app/transfer.rs`).

use bencode_ui::primitives::Tooltip;
use bencode_ui::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};

use crate::app::BenCodeApp;
use crate::app::transfer::{Transfer, switched_to};
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    /// The rule over the message at `ix`, when its turn is another agent's
    /// than the one before.
    pub(super) fn provider_divider(
        &self,
        session: &SessionRow,
        ix: usize,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let harness = switched_to(&session.blocks, ix)?;
        let fg = cx.theme().colors.fg;
        let name = HarnessKind::from_id(harness)
            .map_or(harness.to_string(), |kind| kind.label().to_string());
        let transfer = Transfer::from_block(&session.blocks[ix]);
        let tip = match &transfer {
            Some(transfer) => format!(
                "Continued with {name}. BenCode gave it a recap of the conversation with {}.",
                transfer.from.label()
            ),
            None => format!("Continued with {name}"),
        };
        let rule = || {
            div()
                .h(px(1.0))
                .min_w(px(16.0))
                .flex_1()
                .bg(fg.fill(Fill::Bold))
        };
        let key = &session.blocks[ix].id;
        Some(
            div()
                .id(SharedString::from(format!("provider-divider-{key}")))
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .pt_5()
                .pb_3()
                .tooltip(Tooltip::text(tip))
                .child(rule())
                .child(HarnessIcon::new(harness).size(px(14.0)))
                .children(transfer.map(|transfer| {
                    div()
                        .flex_none()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(transfer.label())
                }))
                .child(rule())
                .into_any_element(),
        )
    }
}
