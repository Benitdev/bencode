//! The project's files for Go to File and search: `bencode_core::files`,
//! here or on the machine a `remote://` project is on.

use std::path::Path;

use bencode_core::remote::protocol::Request;

/// Every file of the project, relative to it (`files::list_project_files`).
/// Blocking: run off the UI thread.
pub fn list_project_files(root: &Path) -> Vec<String> {
    match crate::remote::route(root) {
        Some(route) => route
            .and_then(|route| {
                route.call(Request::ProjectFiles {
                    root: route.host_path.clone(),
                })
            })
            .unwrap_or_else(|err| {
                log::warn!("listing {}: {err}", root.display());
                Vec::new()
            }),
        None => bencode_core::files::list_project_files(root),
    }
}
