//! Open editor files: one handle, dirty flag and edit version per path.

/// One open file and its unsaved-work bookkeeping.
pub struct OpenFile<E> {
    pub path: String,
    pub handle: E,
    dirty: bool,
    version: u64,
    line_count: usize,
    saving: bool,
    resave: bool,
}

impl<E> OpenFile<E> {
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }
}

/// Open files in tab order and the active one.
pub struct OpenFiles<E> {
    files: Vec<OpenFile<E>>,
    active: Option<String>,
}

impl<E> Default for OpenFiles<E> {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            active: None,
        }
    }
}

impl<E> OpenFiles<E> {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &OpenFile<E>> {
        self.files.iter()
    }

    pub fn get(&self, path: &str) -> Option<&OpenFile<E>> {
        self.files.iter().find(|f| f.path == path)
    }

    fn get_mut(&mut self, path: &str) -> Option<&mut OpenFile<E>> {
        self.files.iter_mut().find(|f| f.path == path)
    }

    pub fn active_path(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&OpenFile<E>> {
        self.get(self.active.as_deref()?)
    }

    pub fn is_dirty(&self, path: &str) -> bool {
        self.get(path).is_some_and(OpenFile::is_dirty)
    }

    /// Adds a clean file; an already open path keeps its handle. Activates when nothing is active.
    pub fn insert(&mut self, path: String, handle: E, line_count: usize) -> bool {
        if self.get(&path).is_some() {
            return false;
        }
        if self.active.is_none() {
            self.active = Some(path.clone());
        }
        self.files.push(OpenFile {
            path,
            handle,
            dirty: false,
            version: 0,
            line_count,
            saving: false,
            resave: false,
        });
        true
    }

    /// Makes an open path active; false when it is not open.
    pub fn activate(&mut self, path: &str) -> bool {
        if self.get(path).is_none() {
            return false;
        }
        self.active = Some(path.to_string());
        true
    }

    /// Removes a path; closing the active tab activates its right neighbour, else its left.
    pub fn remove(&mut self, path: &str) -> Option<OpenFile<E>> {
        let ix = self.files.iter().position(|f| f.path == path)?;
        let removed = self.files.remove(ix);
        if self.active.as_deref() == Some(path) {
            self.active = self
                .files
                .get(ix)
                .or_else(|| ix.checked_sub(1).and_then(|prev| self.files.get(prev)))
                .map(|f| f.path.clone());
        }
        Some(removed)
    }

    /// Records an edit: dirty, a new version, and the new line count.
    pub fn mark_changed(&mut self, path: &str, line_count: usize) {
        if let Some(file) = self.get_mut(path) {
            file.dirty = true;
            file.version += 1;
            file.line_count = line_count;
        }
    }

    /// A buffer reloaded from disk: clean, with a new version so an in-flight
    /// save of older text cannot mark it clean again.
    pub fn mark_reloaded(&mut self, path: &str, line_count: usize) {
        if let Some(file) = self.get_mut(path) {
            file.dirty = false;
            file.version += 1;
            file.line_count = line_count;
        }
    }

    pub fn get_handle_mut(&mut self, path: &str) -> Option<&mut E> {
        self.get_mut(path).map(|file| &mut file.handle)
    }

    /// Starts a save and returns the version being written, or queues one if a save is in flight.
    pub fn begin_save(&mut self, path: &str) -> Option<u64> {
        let file = self.get_mut(path)?;
        if file.saving {
            file.resave = true;
            return None;
        }
        file.saving = true;
        Some(file.version)
    }

    /// Ends a save; clears dirty only if nothing was typed since. Returns whether a queued save should run.
    pub fn finish_save(&mut self, path: &str, version: u64, succeeded: bool) -> bool {
        let Some(file) = self.get_mut(path) else {
            return false;
        };
        file.saving = false;
        if succeeded && file.version == version {
            file.dirty = false;
        }
        std::mem::take(&mut file.resave) && file.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> OpenFiles<u8> {
        let mut files = OpenFiles::default();
        files.insert("a".into(), 1, 1);
        files.insert("b".into(), 2, 1);
        files.insert("c".into(), 3, 1);
        files
    }

    #[test]
    fn dirty_is_tracked_per_file() {
        let mut files = three();
        files.mark_changed("b", 7);
        assert!(files.is_dirty("b"));
        assert!(!files.is_dirty("a"));
        assert_eq!(files.get("b").unwrap().line_count(), 7);
        assert!(files.activate("c"));
        assert!(files.is_dirty("b"), "switching tabs keeps edits");
    }

    #[test]
    fn insert_keeps_existing_handle_and_first_becomes_active() {
        let mut files = three();
        assert_eq!(files.active_path(), Some("a"));
        files.mark_changed("a", 2);
        assert!(!files.insert("a".into(), 9, 1));
        assert_eq!(files.get("a").unwrap().handle, 1);
        assert!(files.is_dirty("a"));
        assert!(!files.activate("zzz"));
    }

    #[test]
    fn closing_active_moves_to_neighbour() {
        let mut files = three();
        files.activate("b");
        files.remove("b");
        assert_eq!(files.active_path(), Some("c"));
        files.remove("c");
        assert_eq!(files.active_path(), Some("a"));
        files.remove("a");
        assert_eq!(files.active_path(), None);
        assert!(files.is_empty());
    }

    #[test]
    fn save_clears_dirty_only_when_unchanged() {
        let mut files = three();
        files.mark_changed("a", 1);
        let v = files.begin_save("a").unwrap();
        assert!(!files.finish_save("a", v, true));
        assert!(!files.is_dirty("a"));

        files.mark_changed("a", 1);
        let v = files.begin_save("a").unwrap();
        files.mark_changed("a", 1);
        files.finish_save("a", v, true);
        assert!(files.is_dirty("a"), "edit typed during save stays dirty");
    }

    #[test]
    fn failed_save_stays_dirty_and_concurrent_saves_queue() {
        let mut files = three();
        files.mark_changed("a", 1);
        let v = files.begin_save("a").unwrap();
        assert_eq!(files.begin_save("a"), None);
        files.mark_changed("a", 1);
        assert!(files.finish_save("a", v, true), "queued save should run");
        let v = files.begin_save("a").unwrap();
        assert!(!files.finish_save("a", v, false));
        assert!(files.is_dirty("a"));
        assert!(!files.finish_save("gone", 0, true));
    }
}
