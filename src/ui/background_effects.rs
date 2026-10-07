//! MonoCode `features/settings/model/newThreadBackgroundEffects.worker.ts`:
//! the chat background redrawn as Dither, ASCII, Halftone or Scanlines, one
//! pixel for one. Haze is MonoCode's `.gradient-blur-background` CSS (two
//! blurred copies under vertical masks, fading into the background), which
//! GPUI cannot composite, so it is baked into the image here.
//!
//! Pure pixel work on RGBA bytes; `app/chat_background.rs` runs it on the
//! background executor.

use image::{RgbaImage, imageops};

use crate::ui::appearance::BackgroundEffect;

/// MonoCode `loadSource`: the longest side the artwork is kept at.
pub const MAX_SIDE: u32 = 2048;

/// The decoded artwork: RGBA pixels and each pixel's luma.
pub struct Source {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    luma: Vec<u8>,
}

impl Source {
    /// MonoCode `loadSource`: scaled down to fit [`MAX_SIDE`].
    pub fn new(image: RgbaImage) -> Self {
        let (w, h) = image.dimensions();
        let scale = (MAX_SIDE as f32 / w as f32).min(MAX_SIDE as f32 / h as f32).min(1.0);
        let width = ((w as f32 * scale).round() as u32).max(1);
        let height = ((h as f32 * scale).round() as u32).max(1);
        let image = if (width, height) == (w, h) {
            image
        } else {
            imageops::resize(&image, width, height, imageops::FilterType::Triangle)
        };
        Self::from_pixels(width, height, image.into_raw())
    }

    fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let luma = pixels
            .chunks_exact(4)
            .map(|p| {
                (f32::from(p[0]) * 0.2126 + f32::from(p[1]) * 0.7152 + f32::from(p[2]) * 0.0722)
                    .round() as u8
            })
            .collect();
        Self { width, height, pixels, luma }
    }

    /// MonoCode `coverIndex`: the pixel at (x, y), held inside the image.
    fn index(&self, x: usize, y: usize) -> usize {
        let x = x.min(self.width as usize - 1);
        let y = y.min(self.height as usize - 1);
        y * self.width as usize + x
    }
}

/// The artwork under `effect`, as RGBA. `light` is the theme, for the
/// effects that draw on paper rather than on black.
pub fn render(source: &Source, effect: BackgroundEffect, light: bool) -> Vec<u8> {
    match effect {
        BackgroundEffect::None => source.pixels.clone(),
        BackgroundEffect::Dither => dither(source),
        BackgroundEffect::Ascii => ascii(source, light),
        BackgroundEffect::Halftone => halftone(source, light),
        BackgroundEffect::Scanlines => scanlines(source, light),
        BackgroundEffect::GradientBlur => haze(source),
    }
}

fn byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// MonoCode `ditherPixels`: 2px cells against a 4×4 Bayer matrix, each
/// either pushed to full brightness or sunk to 8%.
fn dither(source: &Source) -> Vec<u8> {
    const BAYER: [[f32; 4]; 4] = [
        [0.0, 8.0, 2.0, 10.0],
        [12.0, 4.0, 14.0, 6.0],
        [3.0, 11.0, 1.0, 9.0],
        [15.0, 7.0, 13.0, 5.0],
    ];
    let (width, height) = (source.width as usize, source.height as usize);
    let mut output = vec![0; source.pixels.len()];
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let base = source.index(x + 1, y + 1) * 4;
            let [r, g, b, a] = [0, 1, 2, 3].map(|c| source.pixels[base + c]);
            let peak = f32::from(r.max(g).max(b));
            let threshold = BAYER[(y / 2) % 4][(x / 2) % 4];
            let gain = if peak / 255.0 > (threshold + 0.5) / 16.0 { 255.0 / peak.max(1.0) } else { 0.08 };
            let color = [byte(f32::from(r) * gain), byte(f32::from(g) * gain), byte(f32::from(b) * gain), a];
            for dy in 0..2.min(height - y) {
                for dx in 0..2.min(width - x) {
                    let at = ((y + dy) * width + x + dx) * 4;
                    output[at..at + 4].copy_from_slice(&color);
                }
            }
        }
    }
    output
}

/// MonoCode `asciiPixels`: a 5×7 glyph per 6×8 cell, denser for more ink,
/// in the cell's colour, laid 40% over the artwork.
fn ascii(source: &Source, light: bool) -> Vec<u8> {
    const GLYPHS: [[u8; 7]; 10] = [
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 4, 0],
        [0, 4, 0, 0, 4, 0, 0],
        [0, 0, 0, 14, 0, 0, 0],
        [0, 0, 14, 0, 14, 0, 0],
        [0, 4, 4, 31, 4, 4, 0],
        [0, 21, 14, 31, 14, 21, 0],
        [10, 10, 31, 10, 31, 10, 10],
        [17, 2, 4, 4, 8, 16, 17],
        [14, 17, 23, 21, 23, 16, 14],
    ];
    let (width, height) = (source.width as usize, source.height as usize);
    let paper = if light { 255.0 } else { 0.0 };
    let mut output = vec![0; source.pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let sample = source.index(x / 6 * 6 + 3, y / 8 * 8 + 4);
            let luma = f32::from(source.luma[sample]);
            let density = if light { 255.0 - luma } else { luma };
            let glyph = GLYPHS[((density / 255.0).sqrt() * 9.0) as usize];
            let ink = x % 6 < 5 && y % 8 < 7 && glyph[y % 8] & (1 << (4 - x % 6)) != 0;
            let pixel = (y * width + x) * 4;
            for color in 0..3 {
                let texture = if ink { f32::from(source.pixels[sample * 4 + color]) } else { paper };
                output[pixel + color] = byte(f32::from(source.pixels[pixel + color]) * 0.6 + texture * 0.4);
            }
            output[pixel + 3] = source.pixels[pixel + 3];
        }
    }
    output
}

/// MonoCode `halftonePixels`: a print dot per 4px cell, larger for more
/// ink, laid 40% over the artwork.
fn halftone(source: &Source, light: bool) -> Vec<u8> {
    let (width, height) = (source.width as usize, source.height as usize);
    let paper = if light { 255.0 } else { 0.0 };
    let mut output = vec![0; source.pixels.len()];
    for y in (0..height).step_by(4) {
        for x in (0..width).step_by(4) {
            let sampled = f32::from(source.luma[source.index(x, y)]);
            let luma = if light { 255.0 - sampled } else { sampled };
            let radius = 2.0 * (0.3 + 0.7 * (luma / 255.0).sqrt());
            let dot = source.index(x + 2, y + 2) * 4;
            for dy in 0..4.min(height - y) {
                for dx in 0..4.min(width - x) {
                    let distance = (dx as f32 - 1.5).hypot(dy as f32 - 1.5);
                    let pixel = ((y + dy) * width + x + dx) * 4;
                    let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0) * (f32::from(source.pixels[dot + 3]) / 255.0);
                    for color in 0..3 {
                        let texture = f32::from(source.pixels[dot + color]) * coverage + paper * (1.0 - coverage);
                        output[pixel + color] = byte(f32::from(source.pixels[pixel + color]) * 0.6 + texture * 0.4);
                    }
                    output[pixel + 3] = source.pixels[pixel + 3];
                }
            }
        }
    }
    output
}

/// MonoCode `scanlinePixels`: every third row at 52%, towards black in the
/// dark theme and towards white in the light one.
fn scanlines(source: &Source, light: bool) -> Vec<u8> {
    let width = source.width as usize;
    let mut output = source.pixels.clone();
    for (y, row) in output.chunks_exact_mut(width * 4).enumerate() {
        if y % 3 != 0 {
            continue;
        }
        for pixel in row.chunks_exact_mut(4) {
            for value in &mut pixel[..3] {
                let v = f32::from(*value);
                *value = byte(if light { v + (255.0 - v) * 0.48 } else { v * 0.52 });
            }
        }
    }
    output
}

/// A piecewise-linear CSS gradient at `t`, from `(position, value)` stops.
fn gradient(stops: &[(f32, f32)], t: f32) -> f32 {
    let Some(&(first, start)) = stops.first() else {
        return 0.0;
    };
    if t <= first {
        return start;
    }
    for pair in stops.windows(2) {
        let ((a, from), (b, to)) = (pair[0], pair[1]);
        if t <= b {
            return from + (to - from) * ((t - a) / (b - a).max(f32::EPSILON));
        }
    }
    stops[stops.len() - 1].1
}

/// MonoCode `.gradient-blur-background`: the artwork sharp at the top, a
/// soft then a strong blur taking over downwards, and all of it fading
/// out. The fade to `--color-background-base` is alpha here, as the pane's
/// background lies under the image.
fn haze(source: &Source) -> Vec<u8> {
    // `--gradient-blur-soft-mask`, `-strong-mask`, `-overlay`, `-mask`.
    const SOFT: [(f32, f32); 2] = [(0.18, 0.0), (0.60, 1.0)];
    const STRONG: [(f32, f32); 2] = [(0.28, 0.0), (0.76, 1.0)];
    const OVERLAY: [(f32, f32); 4] = [(0.16, 0.0), (0.38, 0.26), (0.55, 0.55), (0.75, 0.78)];
    const MASK: [(f32, f32); 7] =
        [(0.19, 1.0), (0.30, 0.94), (0.42, 0.72), (0.54, 0.42), (0.66, 0.16), (0.78, 0.04), (0.90, 0.0)];
    let (width, height) = (source.width, source.height);
    // `blur(7px)` and `blur(18px)` on a pane about 1000px wide.
    let per_px = width.max(height) as f32 / 1000.0;
    let Some(image) = RgbaImage::from_raw(width, height, source.pixels.clone()) else {
        return source.pixels.clone();
    };
    // Blurred small and scaled back: a blur hides the lost detail.
    let (small_w, small_h) = ((width / 4).max(1), (height / 4).max(1));
    let small = imageops::resize(&image, small_w, small_h, imageops::FilterType::Triangle);
    let blurred = |radius: f32| {
        let blurred = imageops::fast_blur(&small, (radius * per_px / 4.0).max(0.5));
        imageops::resize(&blurred, width, height, imageops::FilterType::Triangle).into_raw()
    };
    let (soft, strong) = (blurred(7.0), blurred(18.0));
    let mut output = source.pixels.clone();
    for (y, row) in output.chunks_exact_mut(width as usize * 4).enumerate() {
        let t = (y as f32 + 0.5) / height as f32;
        let (soft_mix, strong_mix) = (gradient(&SOFT, t), gradient(&STRONG, t));
        let alpha = gradient(&MASK, t) * (1.0 - gradient(&OVERLAY, t));
        let start = y * width as usize * 4;
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let at = start + x * 4;
            for color in 0..3 {
                let sharp = f32::from(pixel[color]);
                let softened = sharp + (f32::from(soft[at + color]) - sharp) * soft_mix;
                pixel[color] = byte(softened + (f32::from(strong[at + color]) - softened) * strong_mix);
            }
            pixel[3] = byte(f32::from(pixel[3]) * alpha);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat `width`×`height` image of one colour.
    fn flat(width: u32, height: u32, color: [u8; 4]) -> Source {
        Source::from_pixels(width, height, color.repeat((width * height) as usize))
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn large_artwork_is_scaled_to_fit() {
        let source = Source::new(RgbaImage::from_pixel(4096, 1024, image::Rgba([10, 20, 30, 255])));
        assert_eq!((source.width, source.height), (2048, 512));
        let small = Source::new(RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255])));
        assert_eq!((small.width, small.height), (8, 4));
    }

    #[test]
    fn none_keeps_the_artwork() {
        let source = flat(4, 4, [10, 20, 30, 255]);
        assert_eq!(render(&source, BackgroundEffect::None, false), source.pixels);
    }

    #[test]
    fn dither_lights_or_sinks_each_cell() {
        // Peak 128/255 = 0.50: over the thresholds up to 7, under the rest.
        let source = flat(8, 8, [128, 64, 0, 255]);
        let out = dither(&source);
        assert_eq!(pixel(&out, 8, 0, 0), [255, 128, 0, 255], "threshold 0 is lit");
        assert_eq!(pixel(&out, 8, 1, 1), [255, 128, 0, 255], "cells are 2px");
        assert_eq!(pixel(&out, 8, 2, 0), [10, 5, 0, 255], "threshold 8 sinks to 8%");
    }

    #[test]
    fn scanlines_dim_every_third_row() {
        let source = flat(2, 4, [100, 100, 100, 255]);
        let dark = scanlines(&source, false);
        assert_eq!(pixel(&dark, 2, 0, 0), [52, 52, 52, 255]);
        assert_eq!(pixel(&dark, 2, 0, 1), [100, 100, 100, 255]);
        assert_eq!(pixel(&dark, 2, 0, 3), [52, 52, 52, 255]);
        let light = scanlines(&source, true);
        assert_eq!(pixel(&light, 2, 0, 0), [174, 174, 174, 255]);
    }

    #[test]
    fn ascii_draws_glyphs_over_the_artwork() {
        // White on black: the densest glyph, whose top row is `01110`.
        let source = flat(12, 16, [255, 255, 255, 255]);
        let out = ascii(&source, false);
        assert_eq!(pixel(&out, 12, 0, 0), [153, 153, 153, 255], "no ink: 60% artwork over black");
        assert_eq!(pixel(&out, 12, 1, 0), [255, 255, 255, 255], "ink keeps the cell's colour");
        assert_eq!(pixel(&out, 12, 5, 0), [153, 153, 153, 255], "the gap between glyphs");
    }

    #[test]
    fn halftone_dots_grow_with_ink() {
        let bright = halftone(&flat(8, 8, [255, 255, 255, 255]), false);
        assert_eq!(pixel(&bright, 8, 1, 1), [255, 255, 255, 255], "inside the dot");
        let dim = halftone(&flat(8, 8, [0, 0, 0, 255]), true);
        assert_eq!(pixel(&dim, 8, 1, 1), [0, 0, 0, 255], "light theme inks dark areas");
    }

    #[test]
    fn haze_fades_downwards() {
        let source = flat(16, 40, [200, 100, 50, 255]);
        let out = haze(&source);
        assert_eq!(pixel(&out, 16, 8, 0)[3], 255, "the top stays opaque");
        assert_eq!(pixel(&out, 16, 8, 39)[3], 0, "the bottom is gone");
        let middle = pixel(&out, 16, 8, 20)[3];
        assert!(middle > 0 && middle < 255, "{middle}");
        assert_eq!(out.len(), source.pixels.len());
    }

    #[test]
    fn gradients_interpolate_between_stops() {
        let stops = [(0.2, 0.0), (0.6, 1.0)];
        assert_eq!(gradient(&stops, 0.0), 0.0);
        assert!((gradient(&stops, 0.4) - 0.5).abs() < 1e-6);
        assert_eq!(gradient(&stops, 0.9), 1.0);
    }
}
