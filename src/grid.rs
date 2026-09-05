//! Loading an image and splitting it into a grid of cells.
//!
//! The whole (cropped) image is uploaded to a single `Texture2D`; each grid
//! cell is drawn from a `source` sub-rectangle of that texture, addressed by
//! its cell id (`id = row * cols + col`, i.e. the original index before
//! shuffling).

use macroquad::prelude::*;

pub struct Grid {
    /// Atlas texture containing the cropped source image.
    pub texture: Texture2D,
    /// Grid dimensions (actual, possibly clamped to the image size).
    pub cols: usize,
    pub rows: usize,
    /// Source pixel size of one cell.
    pub cell_w: f32,
    pub cell_h: f32,
    /// Average luminance (0..1) of every cell, indexed by cell id.
    pub luma: Vec<f32>,
}

impl Grid {
    /// Build the cell grid from an image.
    ///
    /// The image is cropped to an integer multiple of the requested grid
    /// dimensions, so every cell covers exactly `cell_width × cell_height`
    /// source pixels.
    pub fn new(img: &image::DynamicImage, cols: usize, rows: usize) -> Grid {
        let (cols, rows, cell_w, cell_h, luma) = cell_layout(img, cols, rows);

        let rgba = img.to_rgba8();
        let eff_w = cell_w * cols;
        let eff_h = cell_h * rows;
        let cropped =
            image::imageops::crop_imm(&rgba, 0, 0, eff_w as u32, eff_h as u32).to_image();
        let texture = Texture2D::from_rgba8(eff_w as u16, eff_h as u16, cropped.as_raw());

        Grid {
            texture,
            cols,
            rows,
            cell_w: cell_w as f32,
            cell_h: cell_h as f32,
            luma,
        }
    }
}

/// Pure (GPU-free) computation of the cell layout and per-cell luminance.
///
/// Returns `(cols, rows, cell_w, cell_h, luma)` where `cell_w/`cell_h` are
/// the source pixel size of a cell and `luma[cell_id]` is the cell's average
/// luminance in `0..1` (`cell_id = row * cols + col`).
pub fn cell_layout(
    img: &image::DynamicImage,
    cols: usize,
    rows: usize,
) -> (usize, usize, usize, usize, Vec<f32>) {
    let rgba = img.to_rgba8();
    let (img_w, img_h) = (rgba.width() as usize, rgba.height() as usize);

    let cols = cols.min(img_w).max(1);
    let rows = rows.min(img_h).max(1);
    let cell_w = (img_w / cols).max(1);
    let cell_h = (img_h / rows).max(1);
    let eff_w = cell_w * cols;
    let eff_h = cell_h * rows;

    let cropped = image::imageops::crop_imm(&rgba, 0, 0, eff_w as u32, eff_h as u32).to_image();

    // Rec. 709 luma weights.
    let luma_of = |r: u8, g: u8, b: u8| 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;

    let mut luma = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let x0 = c * cell_w;
            let y0 = r * cell_h;
            let mut sum = 0.0f32;
            for y in y0..(y0 + cell_h) {
                for x in x0..(x0 + cell_w) {
                    let px = cropped.get_pixel(x as u32, y as u32);
                    sum += luma_of(px[0], px[1], px[2]);
                }
            }
            luma.push(sum / (cell_w * cell_h) as f32 / 255.0);
        }
    }

    (cols, rows, cell_w, cell_h, luma)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small checker of bright and dark cells and checks luma ordering.
    #[test]
    fn luma_ordering_matches_cell_content() {
        let (w, h) = (8u32, 4u32); // base pixels; we will request a 2x1 grid -> 4x4 px cells
        let mut img = image::RgbaImage::new(w, h);

        // Left half bright white, right half solid black.
        for (x, _y, px) in img.enumerate_pixels_mut() {
            let v = if x < 4 { 255u8 } else { 0u8 };
            *px = image::Rgba([v, v, v, 255]);
        }

        let (cols, rows, cell_w, cell_h, luma) =
            cell_layout(&image::DynamicImage::ImageRgba8(img), 2, 1);
        assert_eq!((cols, rows, cell_w, cell_h), (2, 1, 4, 4));
        assert_eq!(luma.len(), 2);
        // Cell 0 (top-left, white) must be brighter than cell 1 (top-right, black).
        assert!(luma[0] > 0.95, "left white cell too dark: {}", luma[0]);
        assert!(luma[1] < 0.05, "right black cell too bright: {}", luma[1]);
        assert!(luma[0] > luma[1]);
    }

    #[test]
    fn grid_sizes_are_clamped_to_image() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(10, 10));
        let (cols, rows, _, _, luma) = cell_layout(&img, 500, 500);
        assert_eq!((cols, rows), (10, 10));
        assert_eq!(luma.len(), 100);
    }
}