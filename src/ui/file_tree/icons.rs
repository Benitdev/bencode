//! File and folder icons: MonoCode `FileTypeIcon`, the Material Icon Theme
//! of its `react-material-icon-theme` package.
//!
//! `assets/file-icons/material-icons.txt` holds the SVGs and the package's
//! lookup tables (see `generate.mjs` beside it). They are multi-coloured, so
//! they are drawn as images: GPUI's `svg()` only paints one colour.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{App, Image, ImageFormat, IntoElement, RenderOnce, Styled, Window, img};

const PACK_TEXT: &str = include_str!("../../../assets/file-icons/material-icons.txt");

/// MonoCode's fallbacks (`resolveFileIcon`, `getFolderIcon`).
const FILE: &str = "file";
const FOLDER: &str = "folder";

struct Pack {
    /// Icon name to its SVG.
    svgs: HashMap<&'static str, &'static str>,
    /// Whole file names, lowercase.
    names: HashMap<&'static str, &'static str>,
    /// Extensions, compound ones included (`d.ts`), lowercase.
    extensions: HashMap<&'static str, &'static str>,
    /// Folder names to their closed icon; `-open` is the open one.
    folders: HashMap<&'static str, &'static str>,
}

static PACK: LazyLock<Pack> = LazyLock::new(|| {
    let mut pack = Pack {
        svgs: HashMap::new(),
        names: HashMap::new(),
        extensions: HashMap::new(),
        folders: HashMap::new(),
    };
    for line in PACK_TEXT.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(tag), Some(key), Some(value)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let table = match tag {
            "I" => &mut pack.svgs,
            "N" => &mut pack.names,
            "E" => &mut pack.extensions,
            "D" => &mut pack.folders,
            _ => continue,
        };
        table.insert(key, value);
    }
    pack
});

/// Decoded once per icon; GPUI keeps the bitmap by the image's id.
static IMAGES: LazyLock<Mutex<HashMap<&'static str, Arc<Image>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The Material icon a file or folder name takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryIcon(&'static str);

impl EntryIcon {
    /// The element, at one of Ely's icon sizes.
    pub fn size(self, size: IconSize) -> EntryIconView {
        EntryIconView { icon: self, size }
    }

    fn image(self) -> Option<Arc<Image>> {
        let svg = PACK.svgs.get(self.0)?;
        let mut images = IMAGES.lock().unwrap_or_else(PoisonError::into_inner);
        Some(
            images
                .entry(self.0)
                .or_insert_with(|| {
                    Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()))
                })
                .clone(),
        )
    }
}

/// An [`EntryIcon`] as drawn.
#[derive(IntoElement)]
pub struct EntryIconView {
    icon: EntryIcon,
    size: IconSize,
}

impl RenderOnce for EntryIconView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = cx.theme().icon_size(self.size);
        // Room is kept either way, as MonoCode holds a blank of the same size.
        match self.icon.image() {
            Some(image) => img(image).size(size).flex_none().into_any_element(),
            None => gpui::div().size(size).flex_none().into_any_element(),
        }
    }
}

/// MonoCode `FileTypeIcon`'s choice for `name`.
pub fn resolve_entry_icon(name: &str, is_dir: bool, is_open: bool) -> EntryIcon {
    EntryIcon(if is_dir {
        folder_icon(name, is_open)
    } else {
        file_icon(name)
    })
}

/// MonoCode `resolveFileIcon`: the whole name, then each compound suffix
/// from the longest (`d.ts` before `ts`), all lowercase.
fn file_icon(name: &str) -> &'static str {
    let key = name.to_lowercase();
    if let Some(icon) = PACK.names.get(key.as_str()) {
        return icon;
    }
    // A leading dot starts the name, not an extension (`.env.local`).
    let body = key.strip_prefix('.').unwrap_or(&key);
    body.match_indices('.')
        .find_map(|(dot, _)| PACK.extensions.get(&body[dot + 1..]).copied())
        .unwrap_or(FILE)
}

/// The package's `getFolderIcon`: the exact name, or the name inside its
/// `.x`, `_x`, `-x` and `__x__` spellings. Case matters, as it does there.
fn folder_icon(name: &str, is_open: bool) -> &'static str {
    let dunder = name
        .strip_prefix("__")
        .and_then(|rest| rest.strip_suffix("__"));
    let bare = name.strip_prefix(['.', '_', '-']);
    let closed = [Some(name), dunder, bare]
        .into_iter()
        .flatten()
        .find_map(|key| PACK.folders.get(key).copied());
    match (closed, is_open) {
        (Some(closed), false) => closed,
        // Every folder icon in the pack has its open twin, under this name.
        (Some(closed), true) => PACK
            .svgs
            .get_key_value(format!("{closed}-open").as_str())
            .map_or(closed, |(open, _)| open),
        (None, false) => FOLDER,
        (None, true) => "folder-open",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What MonoCode's package answers for the same names.
    #[test]
    fn files_take_the_icon_monocode_shows() {
        for (name, icon) in [
            ("App.tsx", "react_ts"),
            ("module-registry.ts", "typescript"),
            ("types.d.ts", "typescript-def"),
            ("package.json", "nodejs"),
            ("Cargo.toml", "toml"),
            ("main.rs", "rust"),
            ("README.md", "readme"),
            ("CLAUDE.md", "markdown"),
            ("Dockerfile", "docker"),
            (".gitignore", "git"),
            (".env.local", "tune"),
            ("foo.test.ts", "test-ts"),
            ("archive.tar.gz", "zip"),
            ("LICENSE", "license"),
            ("a.PNG", "image"),
            ("noext", "file"),
            ("weird.zzzz", "file"),
        ] {
            assert_eq!(
                resolve_entry_icon(name, false, false),
                EntryIcon(icon),
                "{name}"
            );
        }
    }

    #[test]
    fn folders_take_the_icon_monocode_shows() {
        for (name, open, icon) in [
            ("src", false, "folder-src"),
            ("src", true, "folder-src-open"),
            ("components", true, "folder-components-open"),
            (".github", false, "folder-github"),
            (".claude", false, "folder-claude"),
            ("__tests__", false, "folder-test"),
            ("_src", false, "folder-src"),
            ("node_modules", false, "folder-node"),
            ("hooks", false, "folder-hook"),
            ("Src", false, "folder"),
            ("zzz", false, "folder"),
            ("zzz", true, "folder-open"),
        ] {
            assert_eq!(
                resolve_entry_icon(name, true, open),
                EntryIcon(icon),
                "{name}"
            );
        }
    }

    #[test]
    fn every_table_entry_has_its_svg() {
        let pack = &*PACK;
        for icon in [FILE, FOLDER, "folder-open"] {
            assert!(pack.svgs.contains_key(icon), "{icon}");
        }
        for icon in pack.names.values().chain(pack.extensions.values()) {
            assert!(pack.svgs.contains_key(icon), "{icon}");
        }
        for icon in pack.folders.values() {
            assert!(pack.svgs.contains_key(icon), "{icon}");
            assert!(
                pack.svgs.contains_key(format!("{icon}-open").as_str()),
                "{icon}-open"
            );
        }
    }

    /// GPUI draws an SVG image at twice its declared size; each is declared
    /// 16px, so every icon must come out a 32px bitmap, whatever its viewBox.
    #[test]
    fn every_svg_rasterises_at_retina_size() {
        let renderer = gpui::SvgRenderer::new(Arc::new(()));
        for (icon, svg) in &PACK.svgs {
            let image = renderer
                .render_single_frame(svg.as_bytes(), 1.0)
                .unwrap_or_else(|err| panic!("{icon}: {err}"));
            let size = image.size(0);
            assert_eq!((size.width.0, size.height.0), (32, 32), "{icon}");
        }
    }
}
