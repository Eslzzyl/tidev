//! Image preparation for model prompt inputs.
//!
//! Prompt image budgets are based on decoded dimensions and patch count rather
//! than the source image format or whether the image looks like a screenshot.
//! Images that already fit the budget retain their original bytes. Images that
//! exceed the budget are resized and re-encoded in their original format when
//! possible.

use std::io::Cursor;

use ::image::codecs::jpeg::JpegEncoder;
use ::image::imageops::FilterType;
use ::image::{DynamicImage, GenericImageView, ImageFormat};
use anyhow::{Context, Result, bail};

/// The patch size used by the default high-detail image budget.
pub const PROMPT_IMAGE_PATCH_SIZE: u32 = 32;
/// Maximum width or height for the default high-detail prompt image budget.
pub const MAX_PROMPT_IMAGE_DIMENSION: u32 = 2048;
/// Maximum number of patches for the default high-detail prompt image budget.
pub const MAX_PROMPT_IMAGE_PATCHES: usize = 2500;
/// Maximum encoded input size accepted by the image preparer.
pub const MAX_PROMPT_IMAGE_INPUT_BYTES: usize = 32 * 1024 * 1024;

/// A prompt image after applying the local image budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
}

impl PreparedImage {
    pub fn resized(&self) -> bool {
        (self.source_width, self.source_height) != (self.width, self.height)
    }
}

/// Prepare encoded image bytes for a model prompt.
pub fn prepare_prompt_image(data: &[u8], mime: &str) -> Result<PreparedImage> {
    if data.len() > MAX_PROMPT_IMAGE_INPUT_BYTES {
        bail!(
            "image input is {} bytes, exceeding the {} MiB limit",
            data.len(),
            MAX_PROMPT_IMAGE_INPUT_BYTES / (1024 * 1024)
        );
    }

    let format = ::image::guess_format(data).context("failed to detect image format")?;
    if !is_supported_prompt_format(format) {
        bail!("unsupported prompt image format: {format:?}");
    }

    let decoded = ::image::load_from_memory_with_format(data, format)
        .context("failed to decode prompt image")?;
    let (source_width, source_height) = decoded.dimensions();
    let (width, height) = prompt_image_dimensions_for_budget(
        source_width,
        source_height,
        MAX_PROMPT_IMAGE_DIMENSION,
        MAX_PROMPT_IMAGE_PATCHES,
    );

    if (width, height) == (source_width, source_height) {
        return Ok(PreparedImage {
            data: data.to_vec(),
            mime: mime.to_string(),
            source_width,
            source_height,
            width,
            height,
        });
    }

    let resized = decoded.resize(width, height, FilterType::Triangle);
    let data = encode_image(&resized, format)?;
    let mime = mime_for_format(format).to_string();

    Ok(PreparedImage {
        data,
        mime,
        source_width,
        source_height,
        width,
        height,
    })
}

/// Compute output dimensions that satisfy both the dimension and patch limits.
pub fn prompt_image_dimensions_for_budget(
    width: u32,
    height: u32,
    max_dimension: u32,
    max_patches: usize,
) -> (u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let max_dimension = max_dimension.max(1);
    let max_patches = max_patches.max(1);

    if prompt_image_dimensions_fit(width, height, max_dimension, max_patches) {
        return (width, height);
    }

    let max_dimension_scale = (f64::from(max_dimension) / f64::from(width.max(height))).min(1.0);
    let scaled_width = ((f64::from(width) * max_dimension_scale).round() as u32).max(1);
    let scaled_height = ((f64::from(height) * max_dimension_scale).round() as u32).max(1);
    if prompt_image_dimensions_fit(scaled_width, scaled_height, max_dimension, max_patches) {
        return (scaled_width, scaled_height);
    }

    let patch_size = f64::from(PROMPT_IMAGE_PATCH_SIZE);
    let mut scale = (patch_size * patch_size * max_patches as f64
        / f64::from(scaled_width)
        / f64::from(scaled_height))
    .sqrt();
    let scaled_patches_wide = f64::from(scaled_width) * scale / patch_size;
    let scaled_patches_high = f64::from(scaled_height) * scale / patch_size;
    scale *= (scaled_patches_wide.floor() / scaled_patches_wide)
        .min(scaled_patches_high.floor() / scaled_patches_high);

    let mut target_width = ((f64::from(scaled_width) * scale).floor() as u32).max(1);
    let mut target_height = ((f64::from(scaled_height) * scale).floor() as u32).max(1);

    while !prompt_image_dimensions_fit(target_width, target_height, max_dimension, max_patches) {
        if target_width >= target_height && target_width > 1 {
            target_width -= 1;
        } else if target_height > 1 {
            target_height -= 1;
        } else {
            break;
        }
    }

    (target_width, target_height)
}

fn prompt_image_dimensions_fit(
    width: u32,
    height: u32,
    max_dimension: u32,
    max_patches: usize,
) -> bool {
    let patches_wide = width.div_ceil(PROMPT_IMAGE_PATCH_SIZE);
    let patches_high = height.div_ceil(PROMPT_IMAGE_PATCH_SIZE);
    let patch_count = u64::from(patches_wide) * u64::from(patches_high);
    width <= max_dimension && height <= max_dimension && patch_count <= max_patches as u64
}

fn is_supported_prompt_format(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    )
}

fn mime_for_format(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    }
}

fn encode_image(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(Cursor::new(&mut output), 85);
            encoder
                .encode_image(image)
                .context("failed to encode resized JPEG")?;
        }
        ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP => {
            image
                .write_to(&mut Cursor::new(&mut output), format)
                .with_context(|| format!("failed to encode resized {format:?}"))?;
        }
        _ => bail!("unsupported prompt image output format: {format:?}"),
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{DynamicImage, ImageBuffer, Rgba};

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = ImageBuffer::from_pixel(width, height, Rgba([20u8, 40, 60, 255]));
        let mut output = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
            .unwrap();
        output
    }

    #[test]
    fn small_images_keep_original_bytes() {
        let data = png_bytes(64, 32);
        let prepared = prepare_prompt_image(&data, "image/png").unwrap();

        assert_eq!(prepared.data, data);
        assert_eq!((prepared.width, prepared.height), (64, 32));
        assert!(!prepared.resized());
    }

    #[test]
    fn large_images_fit_dimension_and_patch_budget() {
        let data = png_bytes(4096, 2048);
        let prepared = prepare_prompt_image(&data, "image/png").unwrap();

        assert!(prepared.resized());
        assert!(prepared.width <= MAX_PROMPT_IMAGE_DIMENSION);
        assert!(prepared.height <= MAX_PROMPT_IMAGE_DIMENSION);
        let patches = prepared.width.div_ceil(PROMPT_IMAGE_PATCH_SIZE) as usize
            * prepared.height.div_ceil(PROMPT_IMAGE_PATCH_SIZE) as usize;
        assert!(patches <= MAX_PROMPT_IMAGE_PATCHES);
    }

    #[test]
    fn dimensions_handle_vertical_images() {
        assert_eq!(
            prompt_image_dimensions_for_budget(1024, 4096, 2048, 2500),
            (512, 2048)
        );
    }
}
