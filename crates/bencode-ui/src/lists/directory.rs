use gpui::SharedString;
use jiff::Timestamp;

/// One entry of a directory: a folder, or a file with its size; both with when they last changed.
#[derive(Clone, Debug, PartialEq)]
pub struct DirEntry {
    name: SharedString,
    size: Option<u64>,
    modified: Timestamp,
}

impl DirEntry {
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    pub fn is_folder(&self) -> bool {
        self.size.is_none()
    }

    /// A file's size in bytes; none for a folder.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    pub fn modified(&self) -> Timestamp {
        self.modified
    }
}
