//! Images drawn far smaller than the file (a 36px attachment chip over a
//! Retina screenshot). `img(path)` decodes the whole file and uploads it to
//! the GPU at full size, and GPUI keeps both until the app quits: about
//! 60 MB for one 3584×2100 screenshot. A thumbnail is decoded and scaled
//! down off the UI thread, and only it is kept. (No MonoCode counterpart:
//! the WebView scales `<img>` itself.)

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use gpui::{App, Asset, ImageCacheError, ImageSource, Img, RenderImage, img};

/// A project logo on the rail and in its menu (`size-4`, `size-5`), one
/// size so they share a thumbnail.
pub const LOGO: f32 = 20.0;

/// How many device pixels a logical one may take: Retina, then a larger
/// interface scale.
const DENSITY: f32 = 3.0;

/// Loads `(path, longest edge in device pixels)`.
pub enum Thumbnail {}

impl Asset for Thumbnail {
    type Source = (PathBuf, u32);
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        (path, edge): Self::Source,
        _: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            decode(&path, edge)
                .map(Arc::new)
                .map_err(|err| ImageCacheError::Other(Arc::new(err)))
        }
    }
}

/// `path` scaled down (never up) to fit `edge` device pixels, as GPUI
/// draws it (BGRA).
fn decode(path: &std::path::Path, edge: u32) -> anyhow::Result<RenderImage> {
    let image = image::ImageReader::open(path)
        .with_context(|| path.display().to_string())?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("could not decode {}", path.display()))?;
    let mut pixels = if image.width().max(image.height()) > edge {
        image.thumbnail(edge, edge).into_rgba8()
    } else {
        image.into_rgba8()
    };
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(RenderImage::new(vec![image::Frame::new(pixels)]))
}

fn source(path: impl Into<PathBuf>, largest: f32) -> (PathBuf, u32) {
    (path.into(), (largest * DENSITY).ceil() as u32)
}

/// `img` for `path`, decoded no larger than `largest` logical pixels on its
/// longest edge needs.
pub fn thumbnail(path: impl Into<PathBuf>, largest: f32) -> Img {
    let source = source(path, largest);
    img(ImageSource::Custom(Arc::new(move |window, cx| {
        window.use_asset::<Thumbnail>(&source, cx)
    })))
}

/// Forgets the `thumbnail(path, largest)` an image no longer on screen
/// held, in memory and on the GPU. Only for one that was drawn: asking for
/// it starts a load otherwise.
pub fn release(path: impl Into<PathBuf>, largest: f32, cx: &mut App) {
    let source = source(path, largest);
    let image = cx.fetch_asset::<Thumbnail>(&source);
    cx.remove_asset::<Thumbnail>(&source);
    if let Some(Ok(image)) = image {
        // Once the update ends: the window being updated is out of reach
        // until then, and its atlas would keep the texture.
        cx.defer(move |cx| cx.drop_image(image, None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_image_is_scaled_to_the_edge_and_a_small_one_kept() {
        let dir = std::env::temp_dir().join(format!("bencode-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let large = dir.join("large.png");
        image::RgbaImage::from_pixel(400, 200, image::Rgba([255, 0, 0, 255])).save(&large).unwrap();
        let small = dir.join("small.png");
        image::RgbaImage::from_pixel(20, 10, image::Rgba([255, 0, 0, 255])).save(&small).unwrap();

        let thumb = decode(&large, 100).unwrap();
        let size = thumb.size(0);
        assert_eq!((size.width.0, size.height.0), (100, 50));
        // Red stays red once the channels are in GPUI's order.
        assert_eq!(&thumb.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);

        let kept = decode(&small, 100).unwrap().size(0);
        assert_eq!((kept.width.0, kept.height.0), (20, 10));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
