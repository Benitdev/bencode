mod diagram;
mod document;
mod rich;
use gpui::App;

pub use diagram::Diagram;
pub(crate) use diagram::options as svg_options;

/// Binds the documents keys. `init` calls it.
pub(crate) fn bind_keys(cx: &mut App) {
    rich::bind_keys(cx);
    document::bind_keys(cx);
}
