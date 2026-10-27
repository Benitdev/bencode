use gpui::OperatingSystem;

/// Whose conventions the components follow: window buttons, the title bar's layout and key caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// The system this build targets, where the theme starts.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    /// GPUI's name for it, which keystrokes are read and spelled for
    /// (`secondary` is Command on the Mac, Control elsewhere).
    pub fn os(self) -> OperatingSystem {
        match self {
            Platform::Mac => OperatingSystem::Mac,
            Platform::Windows => OperatingSystem::Windows,
            Platform::Linux => OperatingSystem::Linux,
        }
    }
}
