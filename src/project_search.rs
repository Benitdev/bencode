//! Search in files: `bencode_core::search`, here or on the machine a
//! `remote://` project is on (one request: the host reads its own files).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use bencode_core::remote::protocol::Request;
pub use bencode_core::search::{SearchMatch, SearchOptions, SearchResult};

/// Searches `root`; a set `cancel` ends it early with no matches (on a
/// machine, the answer is dropped once it comes).
pub fn search(
    root: &Path,
    options: &SearchOptions,
    cancel: &AtomicBool,
) -> Result<SearchResult, String> {
    let Some(route) = crate::remote::route(root) else {
        return bencode_core::search::search(root, options, cancel);
    };
    if options.query.trim().is_empty() || cancel.load(Ordering::Acquire) {
        return Ok(SearchResult::default());
    }
    let route = route?;
    let found: SearchResult = route.call(Request::Search {
        root: route.host_path.clone(),
        options: options.clone(),
    })?;
    Ok(if cancel.load(Ordering::Acquire) {
        SearchResult::default()
    } else {
        found
    })
}
