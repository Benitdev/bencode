//! File and folder icons, after MonoCode's Material Icon Theme colours,
//! drawn with Lucide glyphs.

use ely_gpui_component::primitives::IconName;
use gpui::{Hsla, rgb};

/// Resolves the icon and color matching MonoCode's Material Icon Theme palette.
pub fn resolve_entry_icon(name: &str, is_dir: bool, is_open: bool) -> (IconName, Hsla) {
    let lower = name.to_lowercase();
    if is_dir {
        if lower == ".agents" {
            return (IconName::Bot, rgb(0xf87171).into());
        }
        if lower == ".claude" {
            return (IconName::Sparkles, rgb(0xf97316).into());
        }
        if lower == ".github" {
            return (IconName::GitBranch, rgb(0xa855f7).into());
        }
        if is_open {
            return (IconName::FolderOpen, rgb(0x60a5fa).into());
        }
        return (IconName::Folder, rgb(0x60a5fa).into());
    }

    if lower == ".gitignore" || lower == ".gitmodules" || lower == ".gitattributes" {
        return (IconName::GitBranch, rgb(0xf97316).into());
    }
    if lower == "readme.md" || lower == "readme" {
        return (IconName::Info, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        return (IconName::FileText, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".json") {
        return (IconName::FileJson, rgb(0xfacc15).into());
    }
    if lower.ends_with(".lock") {
        return (IconName::Lock, rgb(0xa1a1aa).into());
    }
    if lower.ends_with(".rs") {
        return (IconName::FileCode, rgb(0xf97316).into());
    }
    if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        return (IconName::FileCode, rgb(0x38bdf8).into());
    }
    if lower.ends_with(".js") || lower.ends_with(".jsx") {
        return (IconName::FileCode, rgb(0xfacc15).into());
    }
    if lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower == ".env"
    {
        return (IconName::FileCog, rgb(0xeab308).into());
    }
    if lower.ends_with(".sh")
        || lower.ends_with(".bash")
        || lower.ends_with(".zsh")
        || lower == "artisan"
    {
        return (IconName::FileTerminal, rgb(0x4ade80).into());
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".svg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
    {
        return (IconName::FileImage, rgb(0xc084fc).into());
    }

    let ext = lower.rsplit_once('.').map(|(_, ext)| ext);
    match ext {
        Some("py" | "go" | "swift" | "c" | "h" | "cpp" | "java" | "kt" | "rb" | "php" | "css" | "html" | "vue" | "svelte") => {
            (IconName::FileCode, rgb(0x60a5fa).into())
        }
        Some("mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg") => (IconName::FileAudio, rgb(0xf472b6).into()),
        Some("mp4" | "mov" | "mkv" | "webm" | "avi") => (IconName::FileVideoCamera, rgb(0xf472b6).into()),
        Some("zip" | "tar" | "gz" | "tgz" | "rar" | "7z") => (IconName::FileArchive, rgb(0xa1a1aa).into()),
        Some("csv" | "xls" | "xlsx" | "numbers") => (IconName::FileSpreadsheet, rgb(0x4ade80).into()),
        Some("ttf" | "otf" | "woff" | "woff2") => (IconName::FileType, rgb(0xf87171).into()),
        Some("txt" | "rtf" | "pdf" | "doc" | "docx") => (IconName::FileText, rgb(0x94a3b8).into()),
        Some("ini" | "conf") => (IconName::FileCog, rgb(0xeab308).into()),
        _ => (IconName::File, rgb(0x94a3b8).into()),
    }
}

