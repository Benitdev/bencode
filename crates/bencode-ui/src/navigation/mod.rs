mod editor_tabs;
mod overflow;
mod palette;
pub(crate) use palette::marked;
mod tabs;
pub use editor_tabs::{EditorTab, EditorTabs};
pub use overflow::TabOverflowMenu;
pub use palette::SearchPalette;
pub use tabs::{TabPlacement, Tabs};
