mod bars;
mod chrome;
mod columns;
mod status;
mod switcher;
#[cfg(all(test, feature = "test-support"))]
mod tests;
#[cfg(all(test, feature = "test-support"))]
mod title_tests;
mod windows;

pub use bars::{StatusBar, StatusBarItem, Toolbar, ToolbarGroup, ToolbarSeparator};
pub use chrome::{TitleBar, WindowControls, drag_region};
pub use columns::{ColumnShell, ShellColumn};
pub(crate) use status::connectivity_dot;
pub use status::{Connectivity, OfflineIndicator, ZoomControl, ZoomStep};
pub use switcher::WindowSwitcher;
pub use windows::{Corner, Hosted, MiniWindow, WindowManager, open_hosted};
