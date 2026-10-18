//! The Explorer's disk side: `bencode_core::tree` (MonoCode's
//! `src-tauri/src/fs.rs`), run here, or on the machine of a `remote://`
//! project (`Request::Tree`), as MonoCode runs it through the host's
//! `workspace.run`.

use std::path::Path;

use bencode_core::remote::protocol::{Request, TreeOp};
use bencode_core::tree;

use super::FsEntry;

/// `op` on the machine `root` is on; `None` when `root` is here.
fn on_machine<T: serde::de::DeserializeOwned>(
    root: &Path,
    op: impl FnOnce(&crate::remote::Route) -> Result<TreeOp, String>,
) -> Option<Result<T, String>> {
    let route = crate::remote::route(root)?;
    Some(route.and_then(|route| {
        let op = op(&route)?;
        route.call(Request::Tree {
            root: route.host_path.clone(),
            op,
        })
    }))
}

pub fn list_dir(root: &Path, rel_dir: &str) -> Result<Vec<FsEntry>, String> {
    on_machine(root, |_| {
        Ok(TreeOp::List {
            rel: rel_dir.to_string(),
        })
    })
    .unwrap_or_else(|| tree::list_dir(root, rel_dir))
}

pub fn create(root: &Path, parent: &str, name: &str, is_dir: bool) -> Result<String, String> {
    on_machine(root, |_| {
        Ok(TreeOp::Create {
            parent: parent.to_string(),
            name: name.to_string(),
            is_dir,
        })
    })
    .unwrap_or_else(|| tree::create(root, parent, name, is_dir))
}

pub fn rename(root: &Path, rel: &str, name: &str) -> Result<String, String> {
    on_machine(root, |_| {
        Ok(TreeOp::Rename {
            rel: rel.to_string(),
            name: name.to_string(),
        })
    })
    .unwrap_or_else(|| tree::rename(root, rel, name))
}

pub fn delete(root: &Path, rel: &str) -> Result<(), String> {
    on_machine(root, |_| {
        Ok(TreeOp::Delete {
            rel: rel.to_string(),
        })
    })
    .unwrap_or_else(|| tree::delete(root, rel))
}

/// `from` is absolute: inside the project, or (here) anywhere on disk. A
/// project on a machine copies only what is on that machine.
pub fn copy_into(root: &Path, from: &Path, dest_parent: &str) -> Result<String, String> {
    on_machine(root, |route| {
        let from = from
            .to_str()
            .ok_or("This file's name cannot go to a machine.")?;
        let from = route.host_path_of(from).map_err(|_| {
            "Files from this computer cannot be copied into a folder on a machine yet.".to_string()
        })?;
        Ok(TreeOp::CopyInto {
            from,
            dest_parent: dest_parent.to_string(),
        })
    })
    .unwrap_or_else(|| tree::copy_into(root, from, dest_parent))
}

pub fn move_into(root: &Path, rel: &str, dest_parent: &str) -> Result<String, String> {
    on_machine(root, |_| {
        Ok(TreeOp::MoveInto {
            rel: rel.to_string(),
            dest_parent: dest_parent.to_string(),
        })
    })
    .unwrap_or_else(|| tree::move_into(root, rel, dest_parent))
}
