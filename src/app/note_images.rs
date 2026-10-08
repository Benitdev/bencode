//! Images dropped into a note: MonoCode `notes_save_image`
//! (`src-tauri/src/notes.rs`) and `features/notes/noteImages.ts`. A copy
//! of each image is kept under `note-assets/<note id>/` in BenCode's data
//! folder and the note refers to it as `/note-assets/<note id>/<file>`.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};

const NOTE_ASSET_DIR: &str = "note-assets";
/// How a note's markdown names one of its images.
pub const NOTE_IMAGE_PREFIX: &str = "/note-assets/";
const IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];
const NOT_AN_IMAGE: &str = "Drop a PNG, JPG, GIF, WebP, or SVG image.";

/// A stored image: the name it was dropped with and its markdown path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteImage {
    pub name: String,
    pub markdown_path: String,
}

/// Ids land in a path: letters, digits, `-` and `_` only.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// MonoCode `note_image_names`: the file's own name for the alt text, and
/// a name safe to store it under. `None` when it is not an image.
fn image_names(source: &Path) -> Option<(String, String)> {
    let extension = source.extension()?.to_str()?.to_ascii_lowercase();
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }
    let display = source.file_name()?.to_string_lossy().into_owned();
    let stem: String = source
        .file_stem()?
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '-' })
        .take(80)
        .collect();
    let stem = match stem.trim_matches('-') {
        "" => "image",
        stem => stem,
    };
    Some((display, format!("{stem}.{extension}")))
}

/// Whether `path` names an image a note can hold.
pub fn is_note_image(path: &Path) -> bool {
    image_names(path).is_some()
}

/// Copies `source` into note `note_id`'s folder under `data_dir`. Blocking:
/// for the background executor.
fn save_note_image(data_dir: &Path, note_id: &str, source: &Path) -> Result<NoteImage> {
    if !valid_id(note_id) {
        bail!("Invalid note id.");
    }
    let Some((name, safe_name)) = image_names(source) else {
        bail!("Image must be a PNG, JPG, GIF, WebP, or SVG file.");
    };
    let meta = std::fs::metadata(source).with_context(|| source.display().to_string())?;
    if !meta.is_file() {
        bail!("Not a file");
    }
    if meta.len() > IMAGE_MAX_BYTES {
        bail!("Image is too large (maximum {} MB).", IMAGE_MAX_BYTES / 1024 / 1024);
    }
    let dir = data_dir.join(NOTE_ASSET_DIR).join(note_id);
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stored = format!("{stamp}-{safe_name}");
    let destination = dir.join(&stored);
    std::fs::copy(source, &destination).with_context(|| destination.display().to_string())?;
    Ok(NoteImage {
        name,
        markdown_path: format!("{NOTE_IMAGE_PREFIX}{note_id}/{stored}"),
    })
}

/// MonoCode `saveNoteImageAttachments`: stores the images among `paths`.
/// An error when none of them could be added.
pub fn save_note_images(data_dir: &Path, note_id: &str, paths: &[PathBuf]) -> Result<Vec<NoteImage>> {
    let images: Vec<&PathBuf> = paths.iter().filter(|path| image_names(path).is_some()).collect();
    if images.is_empty() {
        bail!(NOT_AN_IMAGE);
    }
    let mut saved = Vec::new();
    let mut failure = None;
    for image in images {
        match save_note_image(data_dir, note_id, image) {
            Ok(image) => saved.push(image),
            Err(err) => failure = failure.or(Some(err)),
        }
    }
    match (saved.is_empty(), failure) {
        (true, Some(err)) => Err(err),
        (true, None) => bail!("None of the dropped images could be added to the note."),
        (false, _) => Ok(saved),
    }
}

/// MonoCode `remove_note_assets`: a deleted note's images go with it.
pub fn remove_note_images(data_dir: &Path, note_id: &str) -> Result<()> {
    if !valid_id(note_id) {
        bail!("Invalid note id.");
    }
    match std::fs::remove_dir_all(data_dir.join(NOTE_ASSET_DIR).join(note_id)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

/// MonoCode `validate_note_asset_path`: the file a note's
/// `/note-assets/<id>/<file>` names, or `None` for any other path.
pub fn note_image_file(data_dir: &Path, markdown_path: &str) -> Option<PathBuf> {
    let relative = Path::new(markdown_path.strip_prefix('/')?);
    let parts: Vec<Component> = relative.components().collect();
    let plain = parts.iter().all(|part| matches!(part, Component::Normal(_)));
    let ours = parts.first() == Some(&Component::Normal(NOTE_ASSET_DIR.as_ref()));
    (plain && ours && parts.len() == 3).then(|| data_dir.join(relative))
}

/// MonoCode `noteImageMarkdown`.
fn image_markdown(image: &NoteImage) -> String {
    let mut alt = String::with_capacity(image.name.len());
    for c in image.name.chars() {
        match c {
            '\r' | '\n' => alt.push(' '),
            '\\' | '[' | ']' => {
                alt.push('\\');
                alt.push(c);
            }
            c => alt.push(c),
        }
    }
    format!("![{alt}]({})", image.markdown_path)
}

/// The blank line a block needs on the side where `text` meets it.
fn padding(text: &str, at_end: bool) -> &'static str {
    let (double, single) = if at_end {
        (text.ends_with("\n\n"), text.ends_with('\n'))
    } else {
        (text.starts_with("\n\n"), text.starts_with('\n'))
    };
    match (text.is_empty(), double, single) {
        (true, ..) | (_, true, _) => "",
        (_, _, true) => "\n",
        _ => "\n\n",
    }
}

/// MonoCode `insertNoteImagesMarkdown`: `value` with `images` in place of
/// bytes `start..end`, as a block of their own, and where the cursor goes.
pub fn insert_images_markdown(
    value: &str,
    start: usize,
    end: usize,
    images: &[NoteImage],
) -> (String, usize) {
    let floor = |at: usize| {
        let mut at = at.min(value.len());
        while !value.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let from = floor(start);
    let to = floor(end).max(from);
    if images.is_empty() {
        return (value.to_string(), from);
    }
    let (before, after) = (&value[..from], &value[to..]);
    let block = images.iter().map(image_markdown).collect::<Vec<_>>().join("\n\n");
    let head = format!("{before}{}{block}", padding(before, true));
    let cursor = head.len() + padding(after, false).len();
    (format!("{head}{}{after}", padding(after, false)), cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bencode-note-images-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn image(name: &str) -> NoteImage {
        NoteImage {
            name: name.into(),
            markdown_path: format!("/note-assets/n1/{name}"),
        }
    }

    #[test]
    fn names_are_safe_and_keep_supported_extensions() {
        let names = image_names(Path::new("/tmp/My shot (1).PNG")).unwrap();
        assert_eq!(names, ("My shot (1).PNG".to_string(), "My-shot--1.png".to_string()));
        assert_eq!(image_names(Path::new("/tmp/---.jpg")).unwrap().1, "image.jpg");
        assert_eq!(image_names(Path::new("/tmp/notes.txt")), None);
        assert_eq!(image_names(Path::new("/tmp/noext")), None);
    }

    #[test]
    fn images_are_copied_named_and_removed_with_the_note() {
        let dir = temp_dir("save");
        let shot = dir.join("shot one.png");
        std::fs::write(&shot, b"png").unwrap();
        let text = dir.join("readme.txt");
        std::fs::write(&text, b"text").unwrap();

        let saved = save_note_images(&dir, "note-1", &[text.clone(), shot]).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "shot one.png");
        let file = note_image_file(&dir, &saved[0].markdown_path).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"png");
        assert!(file.starts_with(dir.join("note-assets/note-1")));

        let none = save_note_images(&dir, "note-1", &[text]).unwrap_err();
        assert_eq!(none.to_string(), NOT_AN_IMAGE);
        assert!(save_note_images(&dir, "../up", &[dir.join("x.png")]).is_err());

        remove_note_images(&dir, "note-1").unwrap();
        assert!(!file.exists());
        remove_note_images(&dir, "note-1").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_notes_own_images_resolve() {
        let dir = Path::new("/data");
        assert_eq!(
            note_image_file(dir, "/note-assets/n1/1-a.png"),
            Some(PathBuf::from("/data/note-assets/n1/1-a.png"))
        );
        for path in [
            "/note-assets/n1/../../secret.png",
            "/note-assets/n1",
            "/other/n1/a.png",
            "note-assets/n1/a.png",
            "/note-assets/n1/deep/a.png",
            "https://x.dev/a.png",
        ] {
            assert_eq!(note_image_file(dir, path), None, "{path}");
        }
    }

    #[test]
    fn images_land_as_a_block_of_their_own() {
        let one = [image("a.png")];
        let md = "![a.png](/note-assets/n1/a.png)";
        assert_eq!(insert_images_markdown("", 0, 0, &one), (md.to_string(), md.len()));
        let (value, cursor) = insert_images_markdown("before after", 7, 7, &one);
        assert_eq!(value, format!("before \n\n{md}\n\nafter"));
        assert_eq!(&value[cursor..], "after");
        // Existing blank lines are not doubled; the selection is replaced.
        let (value, _) = insert_images_markdown("top\n\nXX\nend", 5, 7, &one);
        assert_eq!(value, format!("top\n\n{md}\n\nend"));
        let two = [image("a.png"), image("b [1].png")];
        let (value, _) = insert_images_markdown("x", 99, 99, &two);
        assert!(value.ends_with("\n\n![b \\[1\\].png](/note-assets/n1/b [1].png)"), "{value}");
        assert_eq!(insert_images_markdown("é", 1, 1, &[]), ("é".to_string(), 0));
    }
}
