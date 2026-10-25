use std::{collections::HashMap, sync::LazyLock};

use gpui::{App, Global};

use super::IconName;

/// Which icon a file shows: an owner's rules by name and by extension, laid over Ely's map. Without one, Ely's map alone.
#[derive(Clone, Default)]
pub struct IconTheme {
    names: HashMap<String, IconName>,
    extensions: HashMap<String, IconName>,
}

impl Global for IconTheme {}

impl IconTheme {
    pub(crate) fn file(&self, name: &str) -> IconName {
        let name = name.to_lowercase();
        let extension = name.rsplit_once('.').map(|(_, extension)| extension);
        self.names
            .get(&name)
            .or_else(|| extension.and_then(|extension| self.extensions.get(extension)))
            .copied()
            .unwrap_or_else(|| ely_icon(extension))
    }
}

static ELY: LazyLock<IconTheme> = LazyLock::new(IconTheme::default);

/// The owner's icon theme, or Ely's map alone.
pub(crate) fn icon_theme(cx: &App) -> &IconTheme {
    cx.try_global::<IconTheme>().unwrap_or(&ELY)
}

/// The icon a file's name suggests, under the owner's icon theme.
pub(crate) fn file_icon(name: &str, cx: &App) -> IconName {
    icon_theme(cx).file(name)
}

/// Ely's icon for a file by its extension, in lower case.
fn ely_icon(extension: Option<&str>) -> IconName {
    match extension {
        Some(
            "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "swift" | "c" | "h" | "cpp" | "java"
            | "kt" | "rb" | "php" | "css" | "html",
        ) => IconName::FileCode,
        Some("sh" | "zsh" | "bash" | "command") => IconName::FileTerminal,
        Some("toml" | "yaml" | "yml" | "ini" | "lock" | "env" | "conf") => IconName::FileCog,
        Some("json") => IconName::FileJson,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "bmp" | "tiff") => {
            IconName::FileImage
        }
        Some("mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "aiff") => IconName::FileAudio,
        Some("mp4" | "mov" | "mkv" | "webm" | "avi") => IconName::FileVideoCamera,
        Some("zip" | "tar" | "gz" | "tgz" | "rar" | "7z") => IconName::FileArchive,
        Some("csv" | "xls" | "xlsx" | "numbers") => IconName::FileSpreadsheet,
        Some("ttf" | "otf" | "woff" | "woff2") => IconName::FileType,
        Some("md" | "txt" | "rtf" | "pdf" | "doc" | "docx" | "pages") => IconName::FileText,
        _ => IconName::File,
    }
}

#[cfg(test)]
mod tests {
    use super::{IconName, IconTheme};

    #[test]
    fn a_name_picks_its_icon_by_its_last_extension() {
        assert_eq!(IconTheme::default().file("main.RS"), IconName::FileCode);
        assert_eq!(IconTheme::default().file("lift.PNG"), IconName::FileImage);
        assert_eq!(IconTheme::default().file("notes.md"), IconName::FileText);
        assert_eq!(
            IconTheme::default().file("backup.tar.gz"),
            IconName::FileArchive
        );
        assert_eq!(IconTheme::default().file(".env"), IconName::FileCog);
        assert_eq!(
            IconTheme::default().file("take.mov"),
            IconName::FileVideoCamera
        );
        assert_eq!(IconTheme::default().file("Makefile"), IconName::File);
    }
}
