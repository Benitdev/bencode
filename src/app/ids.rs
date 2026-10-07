//! Ids for rows BenCode creates (MonoCode uses `crypto.randomUUID()`; any
//! unique string works for both apps). The clock keeps them sortable, the
//! counter keeps two made in one millisecond apart.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `{prefix}-{milliseconds}-{counter}`, e.g. `note-1759800000000-3`.
pub fn unique_id(prefix: &str) -> String {
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", super::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_made_together_differ() {
        assert_ne!(unique_id("note"), unique_id("note"));
    }

    #[test]
    fn ids_start_with_their_prefix() {
        assert!(unique_id("note").starts_with("note-"));
    }
}
