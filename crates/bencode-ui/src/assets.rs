use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

/// Icons. Pass to `Application::with_assets`, alone or through `Assets::before`.
#[derive(RustEmbed)]
#[folder = "assets"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|name| name.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

/// bencode-ui's files the source cannot load, each with the reason.
pub(crate) fn missing(source: &dyn AssetSource) -> Vec<String> {
    Assets::iter()
        .filter_map(|path| match source.load(&path) {
            Ok(Some(_)) => None,
            Ok(None) => Some(format!("{path}: not found")),
            Err(error) => Some(format!("{path}: {error}")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gpui::{AssetSource, Result, SharedString};

    use super::{Assets, missing};

    /// A source with none of bencode-ui's files.
    struct Empty;

    impl AssetSource for Empty {
        fn load(&self, _: &str) -> Result<Option<Cow<'static, [u8]>>> {
            Ok(None)
        }

        fn list(&self, _: &str) -> Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn its_own_source_holds_every_file() {
        assert_eq!(missing(&Assets), Vec::<String>::new());
    }

    #[test]
    fn another_source_is_told_what_it_lacks() {
        let missing = missing(&Empty);
        assert!(
            missing
                .iter()
                .any(|line| line.starts_with("icons/check.svg")),
            "{missing:?}"
        );
    }
}
