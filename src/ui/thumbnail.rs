//! Images drawn far smaller than the file (a 36px attachment chip over a
//! Retina screenshot). `img(path)` decodes the whole file and uploads it to
//! the GPU at full size, and GPUI keeps both until the app quits: about
//! 60 MB for one 3584×2100 screenshot. A thumbnail is decoded and scaled
//! down off the UI thread, and only it is kept. (No MonoCode counterpart:
//! the WebView scales `<img>` itself.)

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, Cursor, Seek};
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

    // The future may not borrow `App`, so not `async fn`.
    #[allow(clippy::manual_async_fn)]
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
    let reader = image::ImageReader::open(path).with_context(|| path.display().to_string())?;
    decode_from(reader, edge).with_context(|| format!("could not decode {}", path.display()))
}

fn decode_from(
    reader: image::ImageReader<impl BufRead + Seek>,
    edge: u32,
) -> anyhow::Result<RenderImage> {
    let image = reader.with_guessed_format()?.decode()?;
    let mut pixels = if image.width().max(image.height()) > edge {
        image.thumbnail(edge, edge).into_rgba8()
    } else {
        image.into_rgba8()
    };
    for pixel in pixels.as_chunks_mut::<4>().0 {
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

/// A picture that is not a file here (a blob from git, a file on a
/// machine), known by a hash of its bytes.
#[derive(Clone)]
pub struct ImageBytes {
    hash: u64,
    bytes: Arc<[u8]>,
}

impl ImageBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self {
            hash: hasher.finish(),
            bytes: bytes.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Width and height in pixels, from the header alone; `None` when the
    /// bytes are no picture this build decodes.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        image::ImageReader::new(Cursor::new(&self.bytes[..]))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    }
}

impl std::fmt::Debug for ImageBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImageBytes({:016x}, {} bytes)",
            self.hash,
            self.bytes.len()
        )
    }
}

impl PartialEq for ImageBytes {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.bytes.len() == other.bytes.len()
    }
}

impl Eq for ImageBytes {}

impl Hash for ImageBytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
        self.bytes.len().hash(state);
    }
}

/// Loads `(bytes, longest edge in device pixels)`.
pub enum BytesThumbnail {}

impl Asset for BytesThumbnail {
    type Source = (ImageBytes, u32);
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    #[allow(clippy::manual_async_fn)]
    fn load(
        (image, edge): Self::Source,
        _: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            decode_from(image::ImageReader::new(Cursor::new(&image.bytes[..])), edge)
                .map(Arc::new)
                .map_err(|err| ImageCacheError::Other(Arc::new(err)))
        }
    }
}

thread_local! {
    /// The byte thumbnails drawn so far: only those are let go, since
    /// asking for one that never loaded would start its load.
    static DRAWN_BYTES: RefCell<HashSet<(ImageBytes, u32)>> = RefCell::default();
}

/// `thumbnail` for a picture held in memory.
pub fn thumbnail_bytes(image: &ImageBytes, largest: f32) -> Img {
    let source = (image.clone(), (largest * DENSITY).ceil() as u32);
    img(ImageSource::Custom(Arc::new(move |window, cx| {
        DRAWN_BYTES.with(|drawn| drawn.borrow_mut().insert(source.clone()));
        window.use_asset::<BytesThumbnail>(&source, cx)
    })))
}

/// Forgets `thumbnail_bytes(image, largest)`, if it was drawn.
pub fn release_bytes(image: &ImageBytes, largest: f32, cx: &mut App) {
    let source = (image.clone(), (largest * DENSITY).ceil() as u32);
    if !DRAWN_BYTES.with(|drawn| drawn.borrow_mut().remove(&source)) {
        return;
    }
    let loaded = cx.fetch_asset::<BytesThumbnail>(&source);
    cx.remove_asset::<BytesThumbnail>(&source);
    if let Some(Ok(loaded)) = loaded {
        cx.defer(move |cx| cx.drop_image(loaded, None));
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
        image::RgbaImage::from_pixel(400, 200, image::Rgba([255, 0, 0, 255]))
            .save(&large)
            .unwrap();
        let small = dir.join("small.png");
        image::RgbaImage::from_pixel(20, 10, image::Rgba([255, 0, 0, 255]))
            .save(&small)
            .unwrap();

        let thumb = decode(&large, 100).unwrap();
        let size = thumb.size(0);
        assert_eq!((size.width.0, size.height.0), (100, 50));
        // Red stays red once the channels are in GPUI's order.
        assert_eq!(&thumb.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);

        let bytes = ImageBytes::new(std::fs::read(&large).unwrap());
        assert_eq!(bytes.dimensions(), Some((400, 200)));
        let from_bytes = decode_from(image::ImageReader::new(Cursor::new(&bytes.bytes[..])), 100);
        let size = from_bytes.unwrap().size(0);
        assert_eq!((size.width.0, size.height.0), (100, 50));
        assert_eq!(
            ImageBytes::new(b"not a picture".to_vec()).dimensions(),
            None
        );

        let kept = decode(&small, 100).unwrap().size(0);
        assert_eq!((kept.width.0, kept.height.0), (20, 10));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
