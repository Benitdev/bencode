//! Interface scale (MonoCode `features/settings/model/uiScale.ts`, which
//! zooms the webview). GPUI has no page zoom, so the views write every
//! pixel length through [`px`], which carries the scale, and the window's
//! rem follows for Ely's components (`BenCodeApp::apply_ui_scale`).
//!
//! Lengths the views keep as plain numbers (pane widths, scroll offsets,
//! row heights) are at 100%; [`logical`] brings a measured length back to
//! that space.

use std::sync::atomic::{AtomicU32, Ordering};

use gpui::Pixels;

/// The scale as `f32` bits; 1.0 until the saved one is applied.
static SCALE: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

pub fn ui_scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

pub fn set_ui_scale(scale: f32) {
    SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

/// `gpui::px` at the interface scale.
pub fn px(value: f32) -> Pixels {
    gpui::px(value * ui_scale())
}

/// A length GPUI measured (a pointer position, an element's bounds), as
/// it would be at 100%.
pub fn logical(length: Pixels) -> f32 {
    f32::from(length) / ui_scale()
}
