//! Files attached to a prompt (MonoCode `features/sessions/model/attachments.ts`
//! and `buildClaudeUserMessage`): images travel to Claude as base64 vision
//! blocks; everything else, and every file for argv harnesses, travels as its
//! path for the agent to read from disk.

use std::path::Path;

use base64::Engine;
use serde_json::{Value, json};

/// MonoCode `ATTACHMENT_ONLY_PROMPT`: the turn's text when only files came.
pub const ATTACHMENT_ONLY_PROMPT: &str = "The user attached these files without saying anything. Use the conversation above to work out what they want done with them, then do that. If the conversation gives you nothing to go on, ask.";

/// Images larger than this travel as a path instead of inline data.
const MAX_INLINE_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub size: u64,
    /// Base64 payload for vision images.
    pub data: Option<String>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }

    /// What the transcript keeps: everything but the inline data.
    pub fn to_block_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "path": self.path,
            "mimeType": self.mime_type,
            "kind": if self.is_image() { "image" } else { "file" },
            "size": self.size,
        })
    }
}

/// MonoCode `MIME_BY_EXT` for the kinds that matter to the harnesses.
pub fn mime_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "md" | "txt" | "rs" | "ts" | "tsx" | "js" | "py" | "toml" | "yaml" | "yml" => "text/plain",
        _ if path.is_dir() => "inode/directory",
        _ => "application/octet-stream",
    }
}

/// Reads `path` into an attachment; vision images get their data inline.
/// Blocking: call on the background executor.
pub fn load(path: &Path, id: String) -> std::io::Result<Attachment> {
    let meta = std::fs::metadata(path)?;
    let mime = mime_for(path);
    let vision = matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    );
    let data = if vision && meta.len() <= MAX_INLINE_IMAGE_BYTES {
        Some(base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?))
    } else {
        None
    };
    Ok(Attachment {
        id,
        name: path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        path: path.display().to_string(),
        mime_type: mime.to_string(),
        size: meta.len(),
        data,
    })
}

/// MonoCode `attachmentPathText`.
pub fn path_text(file: &Attachment) -> String {
    let quoted = serde_json::to_string(&file.path).unwrap_or_else(|_| file.path.clone());
    if file.mime_type == "inode/directory" {
        format!("Attached folder (list or read the files inside from this path): {quoted}")
    } else {
        format!("Attached file (read from disk): {quoted}")
    }
}

/// MonoCode `promptText`: the text, or a stand-in when only files came.
pub fn prompt_text(text: &str, files: &[Attachment]) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() && !files.is_empty() {
        ATTACHMENT_ONLY_PROMPT.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Claude's stream-json content: text, then an image block per vision
/// image and a path line for every other file.
pub fn claude_content(text: &str, files: &[Attachment]) -> Vec<Value> {
    let text = prompt_text(text, files);
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    for file in files {
        content.push(match &file.data {
            Some(data) => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": file.mime_type, "data": data },
            }),
            None => json!({ "type": "text", "text": path_text(file) }),
        });
    }
    content
}

/// For harnesses that take the prompt on argv: the text, then a path line
/// per file.
pub fn plain_prompt(text: &str, files: &[Attachment]) -> String {
    std::iter::once(prompt_text(text, files))
        .chain(files.iter().map(path_text))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, mime: &str, data: Option<&str>) -> Attachment {
        Attachment {
            id: name.into(),
            name: name.into(),
            path: format!("/tmp/{name}"),
            mime_type: mime.into(),
            size: 3,
            data: data.map(Into::into),
        }
    }

    #[test]
    fn claude_gets_images_inline_and_files_as_paths() {
        let files = [
            file("a.png", "image/png", Some("QUJD")),
            file("b.pdf", "application/pdf", None),
        ];
        let content = claude_content("look", &files);
        assert_eq!(content[0]["text"], "look");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["data"], "QUJD");
        assert_eq!(
            content[2]["text"],
            "Attached file (read from disk): \"/tmp/b.pdf\""
        );
    }

    #[test]
    fn files_alone_get_the_stand_in_prompt() {
        let files = [file("b.txt", "text/plain", None)];
        assert!(plain_prompt("  ", &files).starts_with(ATTACHMENT_ONLY_PROMPT));
        assert_eq!(plain_prompt("hi", &[]), "hi");
    }

    #[test]
    fn mime_follows_the_extension() {
        assert_eq!(mime_for(Path::new("x.JPG")), "image/jpeg");
        assert_eq!(mime_for(Path::new("x.unknown")), "application/octet-stream");
    }
}
