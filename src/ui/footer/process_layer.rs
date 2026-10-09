//! BenCode's CPU and memory over the footer (no MonoCode counterpart). A
//! sibling of the app under `WindowRoot`, like `RunnerLayer`: a sample
//! redraws this layer, not the app. The app only leaves an empty slot in its
//! footer and measures it; the readout is laid out over that slot as the
//! frame is prepainted, after the app measured it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, AvailableSpace, Context, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, canvas, div, point, prelude::*, size,
};

use crate::app::process_monitor::{ProcessMonitor, SAMPLE_EVERY};
use crate::process_stats;
use crate::ui::composer::runner::Rect;
use crate::ui::scale::px;

/// The readout's row height, the footer's own.
const HEIGHT: f32 = 20.0;

pub struct ProcessLayer {
    /// The footer's slot as the app last measured it; `None` while the
    /// footer is not drawn.
    slot: Rc<Cell<Option<Rect>>>,
    monitor: ProcessMonitor,
    /// Whether the window was active when last drawn or last (de)activated.
    active: bool,
    /// A sample changed the text while the window was inactive.
    stale: bool,
    _activation: Subscription,
}

impl ProcessLayer {
    pub fn new(slot: Rc<Cell<Option<Rect>>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |layer, window, cx| {
            layer.active = window.is_window_active();
            if layer.active && std::mem::take(&mut layer.stale) {
                cx.notify();
            }
        });
        cx.spawn(async move |this, cx| {
            loop {
                let sample = cx
                    .background_executor()
                    .spawn(async { process_stats::sample().map(|sample| (Instant::now(), sample)) })
                    .await;
                let Some((at, sample)) = sample else {
                    return; // unsupported platform
                };
                let landed = this.update(cx, |layer, cx| {
                    if layer.monitor.record(at, sample) {
                        // Nobody reads the readout of a window in the back.
                        if layer.active {
                            cx.notify();
                        } else {
                            layer.stale = true;
                        }
                    }
                });
                if let Err(err) = landed {
                    log::debug!("process sample after layer drop: {err:#}");
                    return;
                }
                cx.background_executor().timer(SAMPLE_EVERY).await;
            }
        })
        .detach();
        Self {
            slot,
            monitor: ProcessMonitor::default(),
            active: window.is_window_active(),
            stale: false,
            _activation: activation,
        }
    }
}

impl Render for ProcessLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.active = window.is_window_active();
        let fg = cx.theme().colors.fg;
        let (cpu, memory) = (self.monitor.cpu.clone(), self.monitor.memory.clone());
        let slot = self.slot.clone();
        canvas(
            move |_, window, cx| -> Option<AnyElement> {
                let slot = slot.get()?;
                if cpu.is_none() && memory.is_none() {
                    return None;
                }
                let stat = |icon: IconName, text: String| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.0))
                        .child(Icon::new(icon).size(IconSize::Xs).color(fg.opacity(0.4)))
                        .child(text)
                };
                let width = px(slot.right - slot.left);
                let mut readout = div()
                    .w(width)
                    .h(px(HEIGHT))
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.55))
                    .child(
                        div()
                            .id("footer-process-stats")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(10.0))
                            .children(cpu.map(|cpu| stat(IconName::Cpu, cpu)))
                            .children(memory.map(|memory| stat(IconName::MemoryStick, memory)))
                            .tooltip(Tooltip::text("BenCode CPU (one core is 100%) and memory")),
                    )
                    .into_any_element();
                readout.prepaint_as_root(
                    point(px(slot.left), px(slot.top)),
                    size(
                        AvailableSpace::Definite(width),
                        AvailableSpace::Definite(px(HEIGHT)),
                    ),
                    window,
                    cx,
                );
                Some(readout)
            },
            |_, readout, window, cx| {
                if let Some(mut readout) = readout {
                    readout.paint(window, cx);
                }
            },
        )
        .absolute()
    }
}
