//! MonoCode's sidebar session folders (`sessionFolders.ts`): named groups
//! of a project's threads, each thread in at most one folder, empty folders
//! dropped. MonoCode keeps them per project in localStorage; BenCode keeps
//! them per project in `settings.json`.

use std::hash::{BuildHasher, RandomState};

use gpui::Context;
use serde::{Deserialize, Serialize};

use crate::app::{BenCodeApp, now_ms};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionFolder {
    pub id: String,
    pub name: String,
    pub session_ids: Vec<String>,
    pub collapsed: bool,
    /// MonoCode `colorIndex` into `FOLDER_COLORS`; 0 or none is untinted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_index: Option<usize>,
    /// MonoCode `customColor` (`#rrggbb`), ahead of `color_index`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_color: Option<String>,
}

/// MonoCode `TAB_GROUP_COLORS` as (hue°, saturation, lightness); the
/// first is the untinted default.
pub const FOLDER_COLORS: [(f32, f32, f32); 9] = [
    (210.0, 0.08, 0.58),
    (211.0, 0.92, 0.62),
    (12.0, 0.80, 0.58),
    (45.0, 0.90, 0.55),
    (142.0, 0.55, 0.50),
    (330.0, 0.70, 0.62),
    (280.0, 0.55, 0.62),
    (175.0, 0.55, 0.48),
    (25.0, 0.85, 0.58),
];

pub fn palette_color(index: usize) -> Option<gpui::Hsla> {
    let (h, s, l) = *FOLDER_COLORS.get(index)?;
    Some(gpui::hsla(h / 360.0, s, l, 1.0))
}

/// `#rrggbb` only (MonoCode `parseCustomHex`).
pub fn parse_hex(text: &str) -> Option<gpui::Hsla> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    gpui::Rgba::try_from(text).ok().map(Into::into)
}

/// `#rrggbb` for a colour.
pub fn to_hex(color: gpui::Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(rgba.r), byte(rgba.g), byte(rgba.b))
}

impl SessionFolder {
    /// MonoCode `folderAccent`: the custom colour, else a palette colour
    /// other than the first.
    pub fn accent(&self) -> Option<gpui::Hsla> {
        if let Some(color) = self.custom_color.as_deref().and_then(parse_hex) {
            return Some(color);
        }
        self.color_index
            .filter(|i| (1..FOLDER_COLORS.len()).contains(i))
            .and_then(palette_color)
    }
}

/// MonoCode `SessionFolderTarget`: where `/add-to-folder` puts a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderTarget {
    Existing(String),
    New(String),
}

fn new_folder_id() -> String {
    let token = RandomState::new().hash_one(now_ms());
    format!("folder-{token:016x}")
}

/// MonoCode `folderContaining`.
pub fn folder_of<'a>(folders: &'a [SessionFolder], session_id: &str) -> Option<&'a SessionFolder> {
    folders
        .iter()
        .find(|f| f.session_ids.iter().any(|id| id == session_id))
}

/// MonoCode `uniqueFolderName`: "New folder", then "New folder 2", …
pub fn unique_folder_name(folders: &[SessionFolder]) -> String {
    const BASE: &str = "New folder";
    let taken = |name: &str| folders.iter().any(|f| f.name == name);
    if !taken(BASE) {
        return BASE.to_string();
    }
    (2..)
        .map(|n| format!("{BASE} {n}"))
        .find(|name| !taken(name))
        .expect("a free name exists")
}

/// `folders` without `session_id` anywhere, and without emptied folders.
pub fn remove_session(folders: &[SessionFolder], session_id: &str) -> Vec<SessionFolder> {
    folders
        .iter()
        .cloned()
        .map(|mut f| {
            f.session_ids.retain(|id| id != session_id);
            f
        })
        .filter(|f| !f.session_ids.is_empty())
        .collect()
}

/// MonoCode `placeSessionInFolder`: into an existing folder (opened), or a
/// new one named `name`, leaving any other folder.
pub fn place_session(
    folders: &[SessionFolder],
    session_id: &str,
    target: &FolderTarget,
) -> Vec<SessionFolder> {
    match target {
        FolderTarget::Existing(folder_id) => {
            if !folders.iter().any(|f| &f.id == folder_id) {
                return folders.to_vec();
            }
            let mut next = remove_session(folders, session_id);
            // The target may have emptied out; it then comes back.
            match next.iter_mut().find(|f| &f.id == folder_id) {
                Some(folder) => {
                    folder.session_ids.push(session_id.to_string());
                    folder.collapsed = false;
                }
                None => {
                    let mut folder = folders
                        .iter()
                        .find(|f| &f.id == folder_id)
                        .cloned()
                        .expect("checked above");
                    folder.session_ids = vec![session_id.to_string()];
                    folder.collapsed = false;
                    next.insert(0, folder);
                }
            }
            next
        }
        FolderTarget::New(name) => {
            let name = name.trim();
            if name.is_empty() {
                return folders.to_vec();
            }
            let mut next = remove_session(folders, session_id);
            next.insert(
                0,
                SessionFolder {
                    id: new_folder_id(),
                    name: name.to_string(),
                    session_ids: vec![session_id.to_string()],
                    collapsed: false,
                    ..SessionFolder::default()
                },
            );
            next
        }
    }
}

/// MonoCode `createFolderWithSessions`: a new open folder holding
/// `session_ids` (taken out of any other folder), first in the list.
pub fn create_folder(folders: &[SessionFolder], session_ids: &[String]) -> (Vec<SessionFolder>, Option<String>) {
    let mut ids: Vec<String> = Vec::new();
    for id in session_ids {
        if !id.is_empty() && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    if ids.is_empty() {
        return (folders.to_vec(), None);
    }
    let mut next = folders.to_vec();
    for id in &ids {
        next = remove_session(&next, id);
    }
    let id = new_folder_id();
    let folder = SessionFolder {
        id: id.clone(),
        name: unique_folder_name(&next),
        session_ids: ids,
        collapsed: false,
        ..SessionFolder::default()
    };
    next.insert(0, folder);
    (next, Some(id))
}

/// MonoCode's picker rows: folders whose name holds the query, then
/// "Create “name”" unless a folder already has exactly that name.
pub fn picker_rows(folders: &[SessionFolder], query: &str) -> Vec<FolderTarget> {
    let name = query.trim();
    let needle = name.to_lowercase();
    let mut rows: Vec<FolderTarget> = folders
        .iter()
        .filter(|f| f.name.to_lowercase().contains(&needle))
        .map(|f| FolderTarget::Existing(f.id.clone()))
        .collect();
    let exact = folders.iter().any(|f| f.name.to_lowercase() == needle);
    if !name.is_empty() && !exact {
        rows.push(FolderTarget::New(name.to_string()));
    }
    rows
}

impl BenCodeApp {
    /// The current project's folders.
    pub fn project_folders(&self) -> &[SessionFolder] {
        self.session_folders
            .get(&self.current_cwd)
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn set_project_folders(&mut self, folders: Vec<SessionFolder>, cx: &mut Context<Self>) {
        let project = self.current_cwd.clone();
        if folders.is_empty() {
            self.session_folders.remove(&project);
        } else {
            self.session_folders.insert(project, folders);
        }
        self.save_settings(cx);
        cx.notify();
    }

    pub fn place_session_in_folder(
        &mut self,
        session_id: &str,
        target: &FolderTarget,
        cx: &mut Context<Self>,
    ) {
        let next = place_session(self.project_folders(), session_id, target);
        self.set_project_folders(next, cx);
    }

    /// The thread menu's "New folder" and a card dropped on a card: a
    /// folder of these threads. Returns its id.
    pub fn new_folder_with_sessions(
        &mut self,
        session_ids: &[String],
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (next, id) = create_folder(self.project_folders(), session_ids);
        id.is_some().then(|| self.set_project_folders(next, cx));
        id
    }

    /// MonoCode "Add to {folder}" for every menu target.
    pub fn add_sessions_to_folder(&mut self, session_ids: &[String], folder_id: &str, cx: &mut Context<Self>) {
        let target = FolderTarget::Existing(folder_id.to_string());
        let next = session_ids
            .iter()
            .fold(self.project_folders().to_vec(), |folders, id| place_session(&folders, id, &target));
        self.set_project_folders(next, cx);
    }

    /// MonoCode "Remove from folder(s)".
    pub fn remove_sessions_from_folders(&mut self, session_ids: &[String], cx: &mut Context<Self>) {
        let next = session_ids
            .iter()
            .fold(self.project_folders().to_vec(), |folders, id| remove_session(&folders, id));
        self.set_project_folders(next, cx);
    }

    pub fn toggle_folder(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        let mut next = self.project_folders().to_vec();
        if let Some(folder) = next.iter_mut().find(|f| f.id == folder_id) {
            folder.collapsed = !folder.collapsed;
        }
        self.set_project_folders(next, cx);
    }

    /// MonoCode `renameFolder`: a blank name keeps the old one.
    pub fn rename_folder(&mut self, folder_id: &str, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut next = self.project_folders().to_vec();
        if let Some(folder) = next.iter_mut().find(|f| f.id == folder_id) {
            folder.name = name.to_string();
        }
        self.set_project_folders(next, cx);
    }

    /// MonoCode `setFolderColor`: `None` (or the first swatch) clears the
    /// tint and any custom colour.
    pub fn set_folder_color(&mut self, folder_id: &str, index: Option<usize>, cx: &mut Context<Self>) {
        let mut next = self.project_folders().to_vec();
        if let Some(folder) = next.iter_mut().find(|f| f.id == folder_id) {
            folder.color_index = index.filter(|i| (1..FOLDER_COLORS.len()).contains(i));
            folder.custom_color = None;
        }
        self.set_project_folders(next, cx);
    }

    /// MonoCode `setFolderCustomColor`, while the picker is dragged: shown
    /// at once, saved when the folder menu closes.
    pub fn preview_folder_custom_color(&mut self, folder_id: &str, hex: String, cx: &mut Context<Self>) {
        if parse_hex(&hex).is_none() {
            return;
        }
        if let Some(folder) = self
            .session_folders
            .get_mut(&self.current_cwd)
            .and_then(|folders| folders.iter_mut().find(|f| f.id == folder_id))
            && folder.custom_color.as_deref() != Some(hex.as_str())
        {
            folder.custom_color = Some(hex);
            self.sessions_ui.folder_colors_unsaved = true;
            cx.notify();
        }
    }

    /// MonoCode "Ungroup" (`dissolveFolder`): the threads stay, loose.
    pub fn ungroup_folder(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        let next: Vec<SessionFolder> = self
            .project_folders()
            .iter()
            .filter(|f| f.id != folder_id)
            .cloned()
            .collect();
        self.set_project_folders(next, cx);
    }

    /// A deleted thread leaves whichever folder held it.
    pub fn forget_folder_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let mut changed = false;
        for folders in self.session_folders.values_mut() {
            if folder_of(folders, session_id).is_some() {
                *folders = remove_session(folders, session_id);
                changed = true;
            }
        }
        if changed {
            self.session_folders
                .retain(|_, folders| !folders.is_empty());
            self.save_settings(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, name: &str, ids: &[&str]) -> SessionFolder {
        SessionFolder {
            id: id.into(),
            name: name.into(),
            session_ids: ids.iter().map(|s| s.to_string()).collect(),
            collapsed: true,
            ..SessionFolder::default()
        }
    }

    #[test]
    fn a_thread_lives_in_one_folder() {
        let folders = vec![folder("a", "Bugs", &["s1"]), folder("b", "UI", &["s2"])];
        let next = place_session(&folders, "s1", &FolderTarget::Existing("b".into()));
        assert_eq!(next.len(), 1, "the emptied folder goes");
        assert_eq!(next[0].session_ids, ["s2", "s1"]);
        assert!(!next[0].collapsed, "the target opens");
        let named = place_session(&next, "s3", &FolderTarget::New("  Infra ".into()));
        assert_eq!(named[0].name, "Infra");
        assert_eq!(
            folder_of(&named, "s3").map(|f| f.name.as_str()),
            Some("Infra")
        );
        assert_eq!(
            place_session(&named, "s3", &FolderTarget::New(" ".into())),
            named
        );
    }

    #[test]
    fn folder_accents_prefer_custom_then_palette() {
        let mut f = folder("a", "A", &["s"]);
        assert!(f.accent().is_none());
        f.color_index = Some(0);
        assert!(f.accent().is_none(), "the first swatch is untinted");
        f.color_index = Some(2);
        assert_eq!(f.accent(), palette_color(2));
        f.custom_color = Some("#ff0000".into());
        assert_eq!(to_hex(f.accent().unwrap()), "#ff0000");
        assert!(parse_hex("red").is_none() && parse_hex("#12345").is_none());
    }

    #[test]
    fn names_count_up_and_removal_drops_empty_folders() {
        let folders = vec![folder("a", "New folder", &["s1", "s2"])];
        assert_eq!(unique_folder_name(&folders), "New folder 2");
        assert_eq!(remove_session(&folders, "s2")[0].session_ids, ["s1"]);
        assert!(remove_session(&remove_session(&folders, "s1"), "s2").is_empty());
    }

    #[test]
    fn the_picker_offers_matches_then_a_new_folder() {
        let folders = vec![
            folder("a", "Bugs", &["s1"]),
            folder("b", "Backend", &["s2"]),
        ];
        assert_eq!(picker_rows(&folders, "").len(), 2);
        assert_eq!(
            picker_rows(&folders, "b"),
            [
                FolderTarget::Existing("a".into()),
                FolderTarget::Existing("b".into()),
                FolderTarget::New("b".into())
            ]
        );
        assert_eq!(
            picker_rows(&folders, "bugs"),
            [FolderTarget::Existing("a".into())]
        );
    }
}
