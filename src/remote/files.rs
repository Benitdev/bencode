//! Files on a machine, for the views that read and write one at a time
//! (the editor and its disk check). The Explorer's changes go whole to the
//! host (`Request::Tree`), as do Go to File and search.

use std::io;

use bencode_core::remote::protocol::{B64, EntryKind, FileWire, MetaWire, Request};

use super::Route;

fn routed(path: &str) -> io::Result<Route> {
    match super::route(path) {
        Some(route) => route.map_err(io::Error::other),
        None => Err(io::Error::other(format!("{path} is not on a machine"))),
    }
}

fn not_found(path: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{path}: no such file or folder"),
    )
}

/// What is at `path` on its machine; `None` when nothing is.
pub fn metadata(path: &str) -> io::Result<Option<MetaWire>> {
    let route = routed(path)?;
    route
        .call(Request::Stat {
            path: route.host_path.clone(),
        })
        .map_err(io::Error::other)
}

/// Up to `max` bytes of the file at `path`, and whether there was more.
pub fn read(path: &str, max: u64) -> io::Result<FileWire> {
    let route = routed(path)?;
    route
        .call(Request::ReadFile {
            path: route.host_path.clone(),
            max,
        })
        .map_err(|err| {
            if err.contains("No such file") {
                not_found(path)
            } else {
                io::Error::other(err)
            }
        })
}

/// Writes the whole file through a temporary one beside it, as the
/// editor's `atomic_write` does here.
pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
    let route = routed(path)?;
    route
        .call::<()>(Request::WriteFile {
            path: route.host_path.clone(),
            bytes: B64(bytes.to_vec()),
            create_new: false,
        })
        .map_err(io::Error::other)
}

pub fn is_dir(path: &str) -> bool {
    matches!(
        metadata(path),
        Ok(Some(MetaWire {
            kind: EntryKind::Dir,
            ..
        })) | Ok(Some(MetaWire {
            target_is_dir: true,
            ..
        }))
    )
}
