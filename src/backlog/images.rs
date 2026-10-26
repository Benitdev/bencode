//! Pictures in a Backlog issue. Its Markdown names an attached file by
//! name (`![image][shot.png]`, or `#image(shot.png)` in Backlog's own
//! notation), which only the API fetches, with the key; bencode-ui draws only
//! pictures on disk. Each is saved once under the data folder
//! (`backlog-attachments/<issue>/`) and the Markdown points at the copy.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::{Config, api_url, call, http};

/// A picture can be large; the description waits for it.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// A picture the Markdown names by its attachment's file name.
#[derive(Debug, PartialEq, Eq)]
struct Reference {
    at: Range<usize>,
    alt: String,
    name: String,
}

/// `![alt][name]` and `#image(name)` / `#thumbnail(name)` in `source`.
fn references(source: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = source[from..].find(['!', '#']) {
        let start = from + offset;
        let rest = &source[start..];
        let reference = if let Some(after) = rest.strip_prefix("![") {
            bracketed(after, ']').and_then(|(alt, after)| {
                let (name, after) = bracketed(after.strip_prefix('[')?, ']')?;
                Some((alt, name, rest.len() - after.len()))
            })
        } else {
            ["#image(", "#thumbnail("]
                .iter()
                .find_map(|open| rest.strip_prefix(open))
                .and_then(|after| bracketed(after, ')'))
                .map(|(name, after)| ("image", name, rest.len() - after.len()))
        };
        match reference {
            Some((alt, name, len)) if !name.trim().is_empty() => {
                found.push(Reference {
                    at: start..start + len,
                    alt: alt.to_string(),
                    name: name.trim().to_string(),
                });
                from = start + len;
            }
            _ => from = start + 1,
        }
    }
    found
}

/// The text up to `close` on the same line, and what follows it.
fn bracketed(text: &str, close: char) -> Option<(&str, &str)> {
    let end = text.find([close, '\n'])?;
    (text[end..].starts_with(close)).then(|| (&text[..end], &text[end + close.len_utf8()..]))
}

/// `source` with each picture `saved` finds a file for pointing at it.
fn rewrite(source: &str, mut saved: impl FnMut(&str) -> Option<PathBuf>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    for reference in references(source) {
        let Some(path) = saved(&reference.name) else {
            continue;
        };
        out.push_str(&source[copied..reference.at.start]);
        out.push_str(&format!("![{}]({})", reference.alt, file_url(&path)));
        copied = reference.at.end;
    }
    out.push_str(&source[copied..]);
    out
}

/// A `file://` address with every byte but a path's own `%`-escaped, so
/// spaces and brackets cannot end the Markdown link.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

/// A file name safe to keep: no folders, nothing hidden.
fn safe_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    name.trim_start_matches('.').to_string()
}

/// One issue's attachments, listed on the first picture asked for.
pub(super) struct Pictures<'a> {
    config: &'a Config,
    key: &'a str,
    listed: Option<Vec<(i64, String)>>,
}

impl<'a> Pictures<'a> {
    pub(super) fn new(config: &'a Config, key: &'a str) -> Self {
        Self {
            config,
            key,
            listed: None,
        }
    }

    /// `markdown` with the attachments it shows saved and pointed at.
    pub(super) fn inline(&mut self, markdown: &str) -> String {
        if references(markdown).is_empty() {
            return markdown.to_string();
        }
        rewrite(markdown, |name| self.saved(name))
    }

    /// The saved copy of the attachment named `name` (the latest one).
    fn saved(&mut self, name: &str) -> Option<PathBuf> {
        let listed = self.listed.get_or_insert_with(|| {
            match call(
                self.config,
                None,
                &format!("/issues/{}/attachments", self.key),
                &[],
                &[],
            ) {
                Ok(data) => attachments(&data),
                Err(err) => {
                    log::warn!("backlog: could not list {}'s attachments: {err}", self.key);
                    Vec::new()
                }
            }
        });
        let id = (listed.iter())
            .filter(|(_, listed)| listed == name)
            .map(|(id, _)| *id)
            .max()?;
        let folder = crate::storage::data_dir()?
            .join("backlog-attachments")
            .join(self.key);
        let path = folder.join(format!("{id}-{}", safe_name(name)));
        if path.is_file() {
            return Some(path);
        }
        if let Err(err) = std::fs::create_dir_all(&folder) {
            log::warn!("backlog: could not make the attachments folder: {err}");
            return None;
        }
        let url = api_url(
            self.config,
            &format!("/issues/{}/attachments/{id}", self.key),
            &[],
        );
        match http::download(&url, &[], &path, DOWNLOAD_TIMEOUT) {
            Ok(status) if (200..300).contains(&status) => Some(path),
            // Nothing was saved: the description keeps its own text.
            Ok(status) => {
                log::warn!(
                    "backlog: an attachment of {} answered HTTP {status}",
                    self.key
                );
                None
            }
            // curl names the host at most, never the key in the query.
            Err(err) => {
                log::warn!(
                    "backlog: could not fetch an attachment of {}: {err}",
                    self.key
                );
                None
            }
        }
    }
}

/// Backlog's attachment list: ids and file names.
fn attachments(data: &Value) -> Vec<(i64, String)> {
    (data.as_array().into_iter().flatten())
        .filter_map(|row| {
            Some((
                row.get("id")?.as_i64()?,
                row.get("name")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_found_in_both_notations() {
        let source = "Look: ![image][a b.png] and #image(c.png), #thumbnail(d.jpg).\n\
                      Not these: ![x](https://e.x/y.png) [a][b] #image() ![x]\n[y]";
        let names: Vec<_> = references(source).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["a b.png", "c.png", "d.jpg"]);
    }

    #[test]
    fn named_pictures_point_at_their_copies() {
        let source = "A ![image][shot.png] B #image(gone.png) C";
        let out = rewrite(source, |name| {
            (name == "shot.png").then(|| PathBuf::from("/data/Application Support/1-shot.png"))
        });
        assert_eq!(
            out,
            "A ![image](file:///data/Application%20Support/1-shot.png) B #image(gone.png) C"
        );
    }

    #[test]
    fn saved_names_stay_in_their_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name(".hidden.png"), "hidden.png");
        assert_eq!(safe_name("画面 1.png"), "画面 1.png");
    }

    #[test]
    fn attachments_are_read_from_the_list() {
        let data = serde_json::json!([
            {"id": 8, "name": "a.png", "size": 10},
            {"id": "bad"},
        ]);
        assert_eq!(attachments(&data), [(8, "a.png".to_string())]);
    }
}
