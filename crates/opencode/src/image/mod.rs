//! Image attachment normalization.
//!
//! Mirrors the TypeScript `Image.normalize`: before an image FilePart is sent
//! to the model, resize and recompress it so it fits the configured width,
//! height, and base64 payload limits. Images already within limits pass
//! through untouched.

use base64::Engine;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

use crate::config::Config;
use crate::message::part::FilePart;

const MAX_BASE64_BYTES: u64 = 4_718_592; // 4.5 * 1024 * 1024
const MAX_WIDTH: u32 = 2000;
const MAX_HEIGHT: u32 = 2000;
const AUTO_RESIZE: bool = true;
const JPEG_QUALITIES: [u8; 5] = [80, 85, 70, 55, 40];

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("Image URL must be a base64 data URL")]
    InvalidDataUrl,
    #[error("Image could not be decoded")]
    Decode,
    #[error(
        "Image {width}x{height} with base64 size {bytes} exceeds configured limits and could not be resized below {max_width}x{max_height}/{max} bytes"
    )]
    Size {
        bytes: u64,
        max: u64,
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    },
}

struct Limits {
    auto_resize: bool,
    max_width: u32,
    max_height: u32,
    max_base64_bytes: u64,
}

fn limits(config: Option<&Config>) -> Limits {
    let image = config
        .and_then(|c| c.attachment.as_ref())
        .and_then(|a| a.image.as_ref());
    Limits {
        auto_resize: image.and_then(|i| i.auto_resize).unwrap_or(AUTO_RESIZE),
        max_width: image.and_then(|i| i.max_width).unwrap_or(MAX_WIDTH),
        max_height: image.and_then(|i| i.max_height).unwrap_or(MAX_HEIGHT),
        max_base64_bytes: image
            .and_then(|i| i.max_base64_bytes)
            .unwrap_or(MAX_BASE64_BYTES),
    }
}

/// Resize/recompress an image `FilePart` so it fits the configured size
/// limits. Returns the original part unchanged when it is already within
/// limits, a re-encoded part when resizing succeeds, or an error when the
/// payload cannot be brought under the limits (or resizing is disabled).
pub fn normalize(part: &FilePart, config: Option<&Config>) -> Result<FilePart, ImageError> {
    let limits = limits(config);

    let marker = ";base64,";
    if !part.url.starts_with("data:") {
        return Err(ImageError::InvalidDataUrl);
    }
    let Some(idx) = part.url.find(marker) else {
        return Err(ImageError::InvalidDataUrl);
    };
    let base64 = &part.url[idx + marker.len()..];
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|_| ImageError::Decode)?;
    let decoded = image::load_from_memory(&bytes).map_err(|_| ImageError::Decode)?;

    let orig_w = decoded.width();
    let orig_h = decoded.height();
    // base64 is ASCII, so character length equals byte length.
    let base64_len = base64.len() as u64;

    if orig_w <= limits.max_width
        && orig_h <= limits.max_height
        && base64_len <= limits.max_base64_bytes
    {
        return Ok(part.clone());
    }

    let size_err = || ImageError::Size {
        bytes: base64_len,
        max: limits.max_base64_bytes,
        width: orig_w,
        height: orig_h,
        max_width: limits.max_width,
        max_height: limits.max_height,
    };

    if !limits.auto_resize {
        return Err(size_err());
    }

    for (w, h) in candidate_sizes(orig_w, orig_h, limits.max_width, limits.max_height) {
        let resized = decoded.resize_exact(w, h, FilterType::Lanczos3);
        if let Some((mime, data)) = encode_candidates(&resized, limits.max_base64_bytes) {
            let mut out = part.clone();
            out.mime = mime.to_string();
            out.url = format!("data:{};base64,{}", mime, data);
            return Ok(out);
        }
    }

    Err(size_err())
}

/// Generate the descending sequence of candidate dimensions: first the image
/// scaled to fit within the max bounds, then each successive size shrunk to
/// 75%, stopping once the sequence converges (matching the TS reducer that
/// dedupes and caps at 32 iterations).
fn candidate_sizes(orig_w: u32, orig_h: u32, max_w: u32, max_h: u32) -> Vec<(u32, u32)> {
    let scale = 1.0_f64
        .min(max_w as f64 / orig_w as f64)
        .min(max_h as f64 / orig_h as f64);

    let mut out: Vec<(u32, u32)> = Vec::new();
    for _ in 0..32 {
        let next = match out.last() {
            None => (
                ((orig_w as f64 * scale).round() as u32).max(1),
                ((orig_h as f64 * scale).round() as u32).max(1),
            ),
            Some(&(w, h)) => (
                if w == 1 {
                    1
                } else {
                    ((w as f64 * 0.75).floor() as u32).max(1)
                },
                if h == 1 {
                    1
                } else {
                    ((h as f64 * 0.75).floor() as u32).max(1)
                },
            ),
        };
        // The sequence is monotonically non-increasing, so once it stops
        // changing every later iteration is a duplicate too.
        if out.last() == Some(&next) {
            break;
        }
        out.push(next);
    }
    out
}

/// Try PNG first, then JPEG at descending qualities; return the first encoding
/// whose base64 payload fits within `max_bytes`.
fn encode_candidates(img: &DynamicImage, max_bytes: u64) -> Option<(&'static str, String)> {
    let mut png = Vec::new();
    if img
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .is_ok()
    {
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        if data.len() as u64 <= max_bytes {
            return Some(("image/png", data));
        }
    }

    // JPEG has no alpha channel; flatten to RGB before encoding.
    let rgb = img.to_rgb8();
    for &quality in &JPEG_QUALITIES {
        let mut jpeg = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality);
        if encoder.encode_image(&rgb).is_ok() {
            let data = base64::engine::general_purpose::STANDARD.encode(&jpeg);
            if data.len() as u64 <= max_bytes {
                return Some(("image/jpeg", data));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{MessageID, PartID, SessionID};

    fn data_url_part(img: &DynamicImage) -> FilePart {
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        FilePart {
            id: PartID::new(),
            session_id: SessionID::new(),
            message_id: MessageID::new(),
            mime: "image/png".to_string(),
            filename: Some("test.png".to_string()),
            url: format!("data:image/png;base64,{}", b64),
            source: None,
        }
    }

    #[test]
    fn small_image_passes_through_unchanged() {
        let img = DynamicImage::new_rgba8(16, 16);
        let part = data_url_part(&img);
        let out = normalize(&part, None).unwrap();
        assert_eq!(out.url, part.url);
        assert_eq!(out.mime, part.mime);
    }

    #[test]
    fn oversized_image_is_resized_within_limits() {
        let img = DynamicImage::new_rgb8(4000, 3000);
        let part = data_url_part(&img);
        let out = normalize(&part, None).unwrap();
        assert_ne!(out.url, part.url);
        let base64 = out.url.split(";base64,").nth(1).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width() <= MAX_WIDTH);
        assert!(decoded.height() <= MAX_HEIGHT);
    }

    #[test]
    fn non_data_url_is_rejected() {
        let mut part = data_url_part(&DynamicImage::new_rgba8(4, 4));
        part.url = "https://example.com/x.png".to_string();
        assert!(matches!(
            normalize(&part, None),
            Err(ImageError::InvalidDataUrl)
        ));
    }

    #[test]
    fn candidate_sizes_descend_and_converge() {
        let sizes = candidate_sizes(4000, 3000, 2000, 2000);
        assert_eq!(sizes.first(), Some(&(2000, 1500)));
        // strictly descending until convergence, capped at 32
        assert!(sizes.len() <= 32);
        for pair in sizes.windows(2) {
            assert!(pair[1].0 <= pair[0].0 && pair[1].1 <= pair[0].1);
        }
    }
}
