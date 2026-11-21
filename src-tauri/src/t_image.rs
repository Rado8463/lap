/**
 * Image processing utilities.
 * project: Lap
 * author:  julyx10
 * date:    2024-08-08
 */
use arboard::Clipboard;
use exif::{In, Reader, Tag};
use fast_image_resize as fir;
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, RgbImage};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata as LittleExifMetadata;
use once_cell::sync::Lazy;

use rusqlite::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{t_jxl, t_libraw, t_utils};
use crate::t_raw_display::RawDisplayOptions;

#[derive(Default)]
pub struct CaptureSettings {
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub focal_length: Option<String>,
    pub iso_speed: Option<String>,
}

/// Reads the core capture settings with the same EXIF reader used when image
/// edits preserve metadata. This covers JPEGs that little_exif accepts but
/// kamadak-exif cannot decode.
pub fn read_capture_settings_with_little_exif(file_path: &str) -> CaptureSettings {
    if !is_jpeg_path(file_path) {
        return CaptureSettings::default();
    }

    panic::catch_unwind(AssertUnwindSafe(|| {
        let metadata = match LittleExifMetadata::new_from_path(Path::new(file_path)) {
            Ok(metadata) => metadata,
            Err(_) => return CaptureSettings::default(),
        };

        CaptureSettings {
            exposure_time: metadata
                .get_tag_by_hex(0x829a, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .and_then(format_little_exif_shutter_speed),
            f_number: metadata
                .get_tag_by_hex(0x829d, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .map(|value| format!("f/{value}")),
            focal_length: metadata
                .get_tag_by_hex(0x920a, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .map(|value| format!("{value} mm")),
            iso_speed: metadata
                .get_tag_by_hex(0x8827, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_iso_value)
                .filter(|value| value != "0")
                .or_else(|| {
                    metadata
                        .get_tag_by_hex(0x8833, Some(ExifTagGroup::EXIF))
                        .next()
                        .and_then(little_exif_iso_value)
                        .filter(|value| value != "0")
                }),
        }
    }))
    .unwrap_or_default()
}

fn little_exif_rational_value(tag: &ExifTag) -> Option<f64> {
    let values = match tag {
        ExifTag::ExposureTime(values) | ExifTag::FNumber(values) | ExifTag::FocalLength(values) => {
            values
        }
        _ => return None,
    };
    let value = values.first()?;
    (value.denominator != 0).then(|| value.nominator as f64 / value.denominator as f64)
}

fn little_exif_iso_value(tag: &ExifTag) -> Option<String> {
    match tag {
        ExifTag::ISO(values) => values.first().map(ToString::to_string),
        ExifTag::ISOSpeed(values) => values.first().map(ToString::to_string),
        _ => None,
    }
}

fn format_little_exif_shutter_speed(value: f64) -> Option<String> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    if value >= 1.0 {
        Some(format!("{value} s"))
    } else {
        Some(format!("1/{} s", 1.0 / value))
    }
}

/// Quick probing of image dimensions without loading the entire file
pub fn get_image_dimensions(file_path: &str) -> Result<(u32, u32), String> {
    if t_utils::is_cloud_only_path(file_path) {
        return Err(t_utils::online_only_message(Path::new(file_path)));
    }
    if crate::t_affinity::is_affinity_path(file_path) {
        // The embedded preview dimensions do not describe the document canvas.
        return Ok((0, 0));
    }
    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_dimensions(file_path);
    }

    if crate::t_psd::is_psd_path(file_path) {
        if let Ok(dimensions) = crate::t_psd::read_dimensions(file_path) {
            return Ok(dimensions);
        }
    }

    if is_ffmpeg_backed_image_path(file_path) {
        let metadata = crate::t_video::get_video_metadata(file_path)?;
        return Ok((metadata.width, metadata.height));
    }

    if is_heic_path(file_path) {
        if let Ok(dimensions) = crate::t_heif::get_heif_dimensions(file_path) {
            return Ok(dimensions);
        }
        if let Ok(metadata) = crate::t_video::get_video_metadata(file_path) {
            if metadata.width > 0 && metadata.height > 0 {
                return Ok((metadata.width, metadata.height));
            }
        }
    }

    // Catch potential panics in the third-party imagesize crate
    let result = panic::catch_unwind(|| imagesize::size(file_path));

    match result {
        Ok(Ok(dimensions)) => {
            let width = dimensions.width as u32;
            let height = dimensions.height as u32;

            if crate::t_libraw::is_tiff_path(file_path) {
                if let Ok((raw_width, raw_height)) = crate::t_libraw::get_raw_dimensions(file_path)
                {
                    if raw_width > width || raw_height > height {
                        return Ok((raw_width, raw_height));
                    }
                }
            }

            Ok((width, height))
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            eprintln!("Panic caught while getting dimensions for: {}", file_path);
            Err(
                "Failed to parse image dimensions due to panic (corrupt or invalid file)"
                    .to_string(),
            )
        }
    }
}

fn get_raw_dimensions_from_exif(file_path: &str) -> Result<Option<(u32, u32)>, String> {
    let exif = match read_exif_permissive(file_path) {
        Some(exif) => exif,
        None => return Ok(None),
    };

    let dimension_tag_pairs = [
        (Tag::PixelXDimension, Tag::PixelYDimension),
        (Tag::ImageWidth, Tag::ImageLength),
    ];

    for (width_tag, height_tag) in dimension_tag_pairs {
        let width = exif
            .get_field(width_tag, In::PRIMARY)
            .and_then(|field| field.value.get_uint(0));
        let height = exif
            .get_field(height_tag, In::PRIMARY)
            .and_then(|field| field.value.get_uint(0));

        if let (Some(width), Some(height)) = (width, height) {
            if width > 0 && height > 0 {
                return Ok(Some((width, height)));
            }
        }
    }

    Ok(None)
}

pub fn read_exif_from_bytes_permissive(data: &[u8]) -> Option<exif::Exif> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut cursor = Cursor::new(data);
        if let Some(exif) = read_exif_container_with_recovery(&mut cursor) {
            return Some(exif);
        }
        if let Some(pos) = data.windows(6).position(|w| w == b"Exif\0\0") {
            let exif_start = pos + 6;
            if exif_start < data.len() {
                if let Some(exif) = read_exif_raw_with_recovery(data[exif_start..].to_vec()) {
                    return Some(exif);
                }
            }
        }
        for sig in [b"II\x2a\x00", b"MM\x00\x2a"] {
            if let Some(pos) = data.windows(4).position(|w| w == sig) {
                if let Some(exif) = read_exif_raw_with_recovery(data[pos..].to_vec()) {
                    return Some(exif);
                }
            }
        }
        None
    }))
    .unwrap_or_else(|_| None)
}

fn read_exif_container_with_recovery(cursor: &mut Cursor<&[u8]>) -> Option<exif::Exif> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    read_exif_result_with_recovery(reader.read_from_container(cursor))
}

fn read_exif_raw_with_recovery(data: Vec<u8>) -> Option<exif::Exif> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    read_exif_result_with_recovery(reader.read_raw(data))
}

fn read_exif_result_with_recovery(result: Result<exif::Exif, exif::Error>) -> Option<exif::Exif> {
    result
        .or_else(|error| error.distill_partial_result(|_| {}))
        .ok()
}

/// A very aggressive binary scanner that looks for the EXIF Orientation tag (0x0112)
/// directly in the byte stream. This is used as a final fallback for non-standard devices.
pub fn scan_orientation_binary(data: &[u8]) -> Option<i32> {
    // Orientation tag is 0x0112. In TIFF, it's a Short (3) with Count 1.
    // Little Endian: 12 01 03 00 01 00 00 00 [Value] 00 00 00
    // Big Endian:    01 12 00 03 00 00 00 01 00 [Value] 00 00

    // Little Endian search
    if let Some(pos) = data
        .windows(12)
        .position(|w| w[0..8] == [0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00])
    {
        let val = data[pos + 8] as i32;
        if (1..=8).contains(&val) {
            return Some(val);
        }
    }

    // Big Endian search
    if let Some(pos) = data
        .windows(12)
        .position(|w| w[0..8] == [0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01])
    {
        let val = data[pos + 9] as i32;
        if (1..=8).contains(&val) {
            return Some(val);
        }
    }

    None
}

pub fn read_exif_permissive(file_path: &str) -> Option<exif::Exif> {
    if !t_utils::content_readable(file_path) {
        return None;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use std::io::{Read, Seek};
        let mut file = File::open(file_path).ok()?;
        let mut header = [0u8; 2];
        file.read_exact(&mut header).ok()?;

        if header != [0xFF, 0xD8] {
            return None;
        }

        loop {
            let mut marker = [0u8; 2];
            if file.read_exact(&mut marker).is_err() {
                break;
            }
            if marker[0] != 0xFF {
                break;
            }
            if marker[1] == 0xD9 || marker[1] == 0xDA {
                break;
            }

            let mut len_bytes = [0u8; 2];
            if file.read_exact(&mut len_bytes).is_err() {
                break;
            }
            let len = u16::from_be_bytes(len_bytes) as usize;
            if len < 2 {
                break;
            }
            let segment_len = len - 2;

            if marker[1] == 0xE1 {
                // APP1
                let mut segment_data = vec![0u8; segment_len];
                if file.read_exact(&mut segment_data).is_err() {
                    break;
                }
                if segment_data.starts_with(b"Exif\0\0") {
                    if let Some(exif) = read_exif_raw_with_recovery(segment_data[6..].to_vec()) {
                        return Some(exif);
                    }
                }
            } else {
                if file
                    .seek(std::io::SeekFrom::Current(segment_len as i64))
                    .is_err()
                {
                    break;
                }
            }
        }
        None
    }))
    .unwrap_or_else(|_| None)
    .or_else(|| {
        let mut file = File::open(file_path).ok()?;
        let mut buffer = vec![0u8; 128 * 1024];
        let n = file.read(&mut buffer).unwrap_or(0);
        read_exif_from_bytes_permissive(&buffer[..n])
    })
}

pub fn get_image_orientation(file_path: &str) -> i32 {
    if !t_utils::content_readable(file_path) {
        return 1;
    }
    let data = match File::open(file_path) {
        Ok(mut f) => {
            let mut buf = vec![0u8; 128 * 1024];
            let n = f.read(&mut buf).unwrap_or(0);
            buf.truncate(n);
            buf
        }
        Err(_) => return 1,
    };

    // 1. Try modern logic
    if let Some(exif) = read_exif_from_bytes_permissive(&data) {
        let orient = exif
            .get_field(Tag::Orientation, In::PRIMARY)
            .or_else(|| exif.fields().find(|f| f.tag == Tag::Orientation))
            .and_then(|field| field.value.get_uint(0))
            .map(|value| value as i32);

        if let Some(o) = orient {
            return o;
        }
    }

    // 2. Industry Fallback: Binary Scan
    // This handles K800i and other phones with broken IFD chains
    scan_orientation_binary(&data).unwrap_or(1)
}

fn apply_orientation(img: DynamicImage, orientation: i32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Minimum short edge as a fraction of `thumbnail_size` (1/4).
const MIN_SHORT_EDGE_DIVISOR: u32 = 4;

/// Computes thumbnail dimensions, keeping the short edge >= `thumbnail_size / 4`
/// (never upscaling); the long edge may exceed `thumbnail_size`.
fn compute_thumbnail_dimensions(width: u32, height: u32, thumbnail_size: u32) -> (u32, u32) {
    if width == 0 || height == 0 || thumbnail_size == 0 {
        return (1, 1);
    }

    if width <= thumbnail_size && height <= thumbnail_size {
        return (width.max(1), height.max(1));
    }

    let long = width.max(height) as f32;
    let short = width.min(height) as f32;
    let min_short_edge = (thumbnail_size / MIN_SHORT_EDGE_DIVISOR).max(1) as f32;
    let fit_scale = thumbnail_size as f32 / long;
    let fit_short = short * fit_scale;
    // Keep the short edge at least min_short_edge, but never upscale it.
    let scale = if fit_short < min_short_edge {
        (short.min(min_short_edge)) / short
    } else {
        fit_scale
    };
    let dst_w = ((width as f32) * scale).round().max(1.0) as u32;
    let dst_h = ((height as f32) * scale).round().max(1.0) as u32;
    (dst_w, dst_h)
}

fn encode_jpeg_rgb8(rgb: &image::RgbImage) -> Result<Vec<u8>, String> {
    crate::t_jpeg::encode_rgb8(rgb, 85)
        .map_err(|e| format!("Failed to encode JPEG thumbnail: {}", e))
}

fn resize_rgb_image_to_jpeg(rgb: image::RgbImage, thumbnail_size: u32) -> Result<Vec<u8>, String> {
    let (src_w, src_h) = rgb.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    if src_w == dst_w && src_h == dst_h {
        return encode_jpeg_rgb8(&rgb);
    }

    let src_image =
        fir::images::Image::from_vec_u8(src_w, src_h, rgb.into_raw(), fir::PixelType::U8x3)
            .map_err(|e| format!("Failed to prepare RGB source image for resize: {}", e))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x3);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| format!("Failed to resize RGB thumbnail: {}", e))?;

    let resized = image::RgbImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        .ok_or_else(|| "Failed to build resized RGB image".to_string())?;
    encode_jpeg_rgb8(&resized)
}

/// Colour shown through transparent pixels in thumbnails (matches the viewer default, RGB 90,90,90).
const THUMBNAIL_TRANSPARENCY_BG: u32 = 90;

/// Composites semi-transparent pixels over the thumbnail background colour.
fn flatten_alpha_over_background(img: &mut image::RgbaImage) {
    for px in img.pixels_mut() {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        for c in 0..3 {
            px[c] = ((px[c] as u32 * a + THUMBNAIL_TRANSPARENCY_BG * (255 - a) + 127) / 255) as u8;
        }
        px[3] = 255;
    }
}

fn resize_rgba_image_to_png(
    mut rgba: image::RgbaImage,
    thumbnail_size: u32,
) -> Result<Vec<u8>, String> {
    flatten_alpha_over_background(&mut rgba);
    let (src_w, src_h) = rgba.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    let resized = if src_w == dst_w && src_h == dst_h {
        rgba
    } else {
        let src_image =
            fir::images::Image::from_vec_u8(src_w, src_h, rgba.into_raw(), fir::PixelType::U8x4)
                .map_err(|e| format!("Failed to prepare PNG source image for resize: {}", e))?;
        let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
        let mut resizer = fir::Resizer::new();
        let options = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

        resizer
            .resize(&src_image, &mut dst_image, &options)
            .map_err(|e| format!("Failed to resize PNG thumbnail: {}", e))?;

        image::RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
            .ok_or_else(|| "Failed to build resized PNG thumbnail".to_string())?
    };

    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(resized)
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode PNG thumbnail: {}", e))?;
    Ok(output.into_inner())
}

pub(crate) fn should_use_png_thumbnail(file_path: &str) -> bool {
    if crate::t_affinity::is_affinity_path(file_path) {
        return true;
    }
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "png" | "gif"))
}

pub fn is_jpeg_path(file_path: &str) -> bool {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "jpe" | "jfif"))
        .unwrap_or(false)
}

fn decode_scaled_jpeg_image(
    file_path: &str,
    _orientation: i32,
    thumbnail_size: u32,
) -> Result<Option<DynamicImage>, String> {
    if !is_jpeg_path(file_path) || thumbnail_size == 0 {
        return Ok(None);
    }

    // Logic: We pass the thumbnail size to libjpeg-turbo, which picks the best 1/8, 1/4, 1/2 scale.
    match crate::t_jpeg::decode_rgb8_scaled(file_path, thumbnail_size, thumbnail_size) {
        Ok((pixels, w, h)) => {
            let img = RgbImage::from_raw(w, h, pixels)
                .ok_or_else(|| "Failed to build RGB image from turbo pixels".to_string())?;
            Ok(Some(DynamicImage::ImageRgb8(img)))
        }
        Err(e) => {
            eprintln!(
                "libjpeg-turbo scaled decode failed for {}: {}",
                file_path, e
            );
            Ok(None) // Fallback to standard decode
        }
    }
}

pub(crate) fn resize_dynamic_image_to_jpeg(
    img: DynamicImage,
    orientation: i32,
    thumbnail_size: u32,
) -> Result<Vec<u8>, String> {
    let adjusted = apply_orientation(img, orientation);

    if !adjusted.color().has_alpha() {
        return resize_rgb_image_to_jpeg(adjusted.to_rgb8(), thumbnail_size);
    }

    let mut rgba = adjusted.to_rgba8();
    flatten_alpha_over_background(&mut rgba);
    let (src_w, src_h) = rgba.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    if src_w == dst_w && src_h == dst_h {
        return encode_jpeg_rgb8(&DynamicImage::ImageRgba8(rgba).to_rgb8());
    }

    let src_image =
        fir::images::Image::from_vec_u8(src_w, src_h, rgba.into_raw(), fir::PixelType::U8x4)
            .map_err(|e| format!("Failed to prepare source image for resize: {}", e))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| format!("Failed to resize thumbnail: {}", e))?;

    let resized = image::RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        .ok_or_else(|| "Failed to build resized RGBA image".to_string())?;
    encode_jpeg_rgb8(&DynamicImage::ImageRgba8(resized).to_rgb8())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchThumbnailStats {
    pub processed: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub fn generate_directory_thumbnails(
    dir_path: &str,
    output_dir: &str,
    thumbnail_size: u32,
) -> Result<BatchThumbnailStats, String> {
    let dir_root = Path::new(dir_path);
    let files: Vec<PathBuf> = WalkDir::new(dir_path)
        .into_iter()
        .filter_entry(crate::t_utils::is_visible_or_root)
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();

    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create thumbnail output directory: {}", e))?;

    let processed = files.len();
    let results: Vec<bool> = files
        .iter()
        .map(|path| {
            let path_str = path.to_string_lossy().to_string();
            let Some(file_type) = t_utils::get_file_type(&path_str) else {
                return false;
            };
            if file_type != 1 && file_type != 3 {
                return false;
            }

            let orientation = get_image_orientation(&path_str);
            let thumb = if file_type == 3 {
                get_raw_thumbnail(&path_str, orientation, thumbnail_size, RawDisplayOptions::rendered_bright())
            } else {
                get_image_thumbnail(&path_str, orientation, thumbnail_size)
            };

            let Ok(Some(data)) = thumb else {
                return false;
            };

            let relative_path = path.strip_prefix(dir_root).ok().unwrap_or(path.as_path());
            let output_path = Path::new(output_dir)
                .join(relative_path)
                .with_extension("jpg");
            if let Some(parent) = output_path.parent() {
                if fs::create_dir_all(parent).is_err() {
                    return false;
                }
            }
            fs::write(output_path, data).is_ok()
        })
        .collect();

    let succeeded = results.iter().filter(|ok| **ok).count();
    Ok(BatchThumbnailStats {
        processed,
        succeeded,
        failed: processed.saturating_sub(succeeded),
    })
}

/// Get a thumbnail from an image file path
pub fn get_image_thumbnail(
    file_path: &str,
    orientation: i32,
    thumbnail_size: u32,
) -> Result<Option<Vec<u8>>, String> {
    get_image_thumbnail_with_encoding(file_path, orientation, thumbnail_size, false)
}

pub(crate) fn get_image_thumbnail_with_encoding(
    file_path: &str,
    orientation: i32,
    thumbnail_size: u32,
    force_jpeg: bool,
) -> Result<Option<Vec<u8>>, String> {
    t_utils::require_local_content(Path::new(file_path)).map_err(|e| e.to_string())?;
    if crate::t_affinity::is_affinity_path(file_path) {
        return crate::t_affinity::decode_preview(file_path)
            .map(|img| {
                if force_jpeg {
                    resize_dynamic_image_to_jpeg(img, 1, thumbnail_size)
                } else {
                    resize_rgba_image_to_png(img.to_rgba8(), thumbnail_size)
                }
            })
            .transpose();
    }
    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_thumbnail(file_path, thumbnail_size);
    }

    if crate::t_psd::is_psd_path(file_path) {
        if let Ok(data) = psd_composite_jpeg(file_path, thumbnail_size) {
            return Ok(Some(data));
        }
    }

    if is_ffmpeg_backed_image_path(file_path) {
        return crate::t_video::get_video_thumbnail_sync(file_path, thumbnail_size, None, None);
    }

    if crate::t_libraw::is_tiff_path(file_path) {
        if let Ok(Some(data)) = crate::t_libraw::get_raw_thumbnail(file_path, thumbnail_size, RawDisplayOptions::rendered_bright()) {
            return Ok(Some(data));
        }
    }

    let result = panic::catch_unwind(|| {
        let img =
            if let Some(img) = decode_scaled_jpeg_image(file_path, orientation, thumbnail_size)? {
                img
            } else {
                let img_reader = ImageReader::open(file_path)
                    .map_err(|e| format!("Failed to open image: {}", e))?;

                match img_reader.decode() {
                    Ok(img) => img,
                    Err(e) => {
                        // Some formats/variants (notably AVIF) may fail to decode via `image` depending on
                        // the underlying codec support. On macOS, fall back to `sips` which supports
                        // more system formats and returns a JPEG directly.
                        #[cfg(target_os = "macos")]
                        if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, thumbnail_size) {
                            return Ok(Some(data));
                        }
                        // On other platforms, fall back to the bundled FFmpeg sidecar when available.
                        // This is already used for HEIC/HEIF on non-macOS and tends to support more
                        // real-world AVIF variants than the pure-Rust decode path.
                        #[cfg(not(target_os = "macos"))]
                        {
                            if let Ok(Some(data)) = crate::t_video::get_video_thumbnail_sync(
                                file_path,
                                thumbnail_size,
                                None,
                                None,
                            ) {
                                return Ok(Some(data));
                            }
                        }
                        return Err(format!("Failed to decode image: {}", e));
                    }
                }
            };
        if !force_jpeg && should_use_png_thumbnail(file_path) {
            resize_rgba_image_to_png(
                apply_orientation(img, orientation).to_rgba8(),
                thumbnail_size,
            )
            .map(Some)
        } else {
            resize_dynamic_image_to_jpeg(img, orientation, thumbnail_size).map(Some)
        }
    });

    match result {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "Panic caught while creating image thumbnail for: {}",
                file_path
            );
            Ok(None)
        }
    }
}

fn select_embedded_jpeg(
    file_path: &str,
    thumbnail_size: Option<u32>,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    let mut file = File::open(file_path).map_err(|e| e.to_string())?;
    let mut header = [0; 4];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header == *b"II\x2a\x00" || header == *b"MM\x00\x2a" {
        // TIFF offsets refer to the file, not the bounded metadata buffer.
        let exif = crate::t_embedded_jpeg::read_tiff_metadata(&mut file)?;
        crate::t_embedded_jpeg::select(&mut file, &exif, thumbnail_size)
    } else {
        let exif = match read_exif_permissive(file_path) {
            Some(exif) => exif,
            None => return Ok(None),
        };
        crate::t_embedded_jpeg::select(&mut Cursor::new(exif.buf()), &exif, thumbnail_size)
    }
}

fn select_embedded_jpeg_for_preview(
    file_path: &str,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    select_embedded_jpeg(file_path, None)
}

fn select_embedded_jpeg_for_thumbnail(
    file_path: &str,
    thumbnail_size: u32,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    select_embedded_jpeg(file_path, Some(thumbnail_size))
}

pub fn get_raw_preview_image(
    file_path: &str,
    prefer_embedded_jpeg: RawDisplayOptions,
) -> Result<Option<Vec<u8>>, String> {
    if let Ok(Some(data)) = t_libraw::get_raw_preview_image(file_path, prefer_embedded_jpeg) {
        return Ok(Some(data));
    }

    get_raw_preview_fallback_image(file_path)
}

fn get_raw_preview_fallback_image(file_path: &str) -> Result<Option<Vec<u8>>, String> {
    // Match embedded browsing before falling back to generic TIFF/JPEG extraction,
    // which can expose only the small EXIF thumbnail for some unsupported RAWs.
    if let Some(data) = get_embedded_preview(file_path) {
        return Ok(Some(data));
    }
    // Final fallback for unsupported RAW renderers or files without a usable
    // processed output. The selected source still controls the normal path.
    if let Ok(Some(preview)) = select_embedded_jpeg_for_preview(file_path) {
        // A readable JPEG header does not guarantee a decodable payload.
        // Preserve the remaining fallbacks if the embedded image is damaged.
        if let Ok(image) = image::load_from_memory(&preview.data) {
            let image = apply_orientation(image, preview.orientation);
            let buf = crate::t_jpeg::encode_rgb8(&image.to_rgb8(), 85)
                .map_err(|e| format!("Failed to encode embedded RAW preview: {}", e))?;
            return Ok(Some(buf));
        }
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, 4096) {
        return Ok(Some(data));
    }

    let orientation = get_image_orientation(file_path);

    // Final fallback for formats that can be decoded directly by `image`.
    if let Ok(Some(data)) = get_image_thumbnail(file_path, orientation, 4096) {
        return Ok(Some(data));
    }

    Ok(None)
}

pub fn get_raw_dimensions(file_path: &str) -> Result<(u32, u32), String> {
    if let Ok((width, height, _raw_flip)) = t_libraw::get_raw_dimensions_with_flip(file_path) {
        if width > 0 && height > 0 {
            return Ok((width, height));
        }
    }

    if let Ok((width, height)) = get_image_dimensions(file_path) {
        if width > 0 && height > 0 {
            return Ok((width, height));
        }
    }

    if let Ok(Some((width, height))) = get_raw_dimensions_from_exif(file_path) {
        return Ok((width, height));
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some((width, height))) = get_dimensions_with_sips(file_path) {
        return Ok((width, height));
    }

    if let Ok(Some(preview)) = select_embedded_jpeg_for_preview(file_path) {
        return Ok((preview.width, preview.height));
    }

    Err("Failed to resolve RAW dimensions".to_string())
}

pub fn get_raw_thumbnail(
    file_path: &str,
    orientation: i32,
    thumbnail_size: u32,
    prefer_embedded_jpeg: RawDisplayOptions,
) -> Result<Option<Vec<u8>>, String> {
    if let Ok(Some(data)) = t_libraw::get_raw_thumbnail(
        file_path,
        thumbnail_size,
        prefer_embedded_jpeg,
    ) {
        return Ok(Some(data));
    }

    // Fallback: EXIF-based embedded JPEG extraction
    if let Ok(Some(preview)) = select_embedded_jpeg_for_thumbnail(file_path, thumbnail_size) {
        if let Ok(img) = image::load_from_memory(&preview.data) {
            return resize_dynamic_image_to_jpeg(img, preview.orientation, thumbnail_size).map(Some);
        }
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, thumbnail_size) {
        return Ok(Some(data));
    }

    // Fallback for formats that can be decoded directly by `image`.
    get_image_thumbnail(file_path, orientation, thumbnail_size)
}

/// edit image impl

/// crop data
#[derive(Debug, Deserialize, Serialize)]
pub struct CropData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// resize data
#[derive(Debug, Deserialize, Serialize)]
pub struct ResizeData {
    width: Option<u32>,
    height: Option<u32>,
}

/// edit params
#[derive(Debug, Deserialize, Serialize)]
pub struct EditParams {
    #[serde(rename = "sourceFilePath")]
    source_file_path: String,
    #[serde(rename = "destFilePath")]
    dest_file_path: String,
    #[serde(rename = "outputFormat")]
    output_format: String,
    orientation: i32, // exif orientation value
    #[serde(rename = "flipHorizontal")]
    flip_horizontal: bool,
    #[serde(rename = "flipVertical")]
    flip_vertical: bool,
    rotate: i32,
    #[serde(rename = "cropAngle", default)]
    crop_angle: f32,
    crop: CropData,
    resize: ResizeData,
    quality: Option<u8>,
    // New adjustments
    filter: Option<String>,  // "grayscale", "sepia", "invert"
    brightness: Option<i32>, // -100 to 100
    contrast: Option<f32>,   // -100.0 to 100.0
    blur: Option<f32>,       // sigma > 0
    hue_rotate: Option<i32>, // degrees
    saturation: Option<f32>, // multiplier, 1.0 is normal
}

/// edit an image and save to dest file
pub async fn edit_image(params: EditParams) -> bool {
    if let Ok(img) = get_edited_image(&params).await {
        let path = Path::new(&params.dest_file_path);
        let format = match params.output_format.as_str() {
            "png" => image::ImageFormat::Png,
            "webp" => image::ImageFormat::WebP,
            _ => image::ImageFormat::Jpeg,
        };

        // Snapshot original metadata before overwriting the file.
        // For overwrite (source == dest) we must copy the original to a
        // temp location first — once we File::create the destination the
        // original EXIF is gone. For save-as-new the source is untouched.
        let metadata_backup_path = if format == image::ImageFormat::Jpeg
            || format == image::ImageFormat::WebP
        {
            match prepare_metadata_backup_path(&params.source_file_path, &params.dest_file_path) {
                Ok(path) => path,
                Err(_) => return false,
            }
        } else {
            None
        };

        let metadata_source = metadata_backup_path
            .as_ref()
            .map(|p| p.as_path())
            .unwrap_or_else(|| Path::new(&params.source_file_path));

        let quality = params.quality.unwrap_or(80);
        let save_ok = if format == image::ImageFormat::Jpeg {
            if let Ok(file) = std::fs::File::create(path) {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
                encoder.encode_image(&img).is_ok()
            } else {
                false
            }
        } else {
            img.save_with_format(path, format).is_ok()
        };

        if !save_ok {
            cleanup_metadata_backup(&metadata_backup_path);
            return false;
        }

        if format == image::ImageFormat::Jpeg || format == image::ImageFormat::WebP {
            if let Err(_) = copy_metadata_to_output(metadata_source, path) {
                if metadata_backup_path.is_some() {
                    let _ = fs::copy(metadata_source, path);
                } else {
                    let _ = fs::remove_file(path);
                }
                cleanup_metadata_backup(&metadata_backup_path);
                return false;
            }

            cleanup_metadata_backup(&metadata_backup_path);
        }

        return true;
    }
    false
}

fn prepare_metadata_backup_path(
    source_file_path: &str,
    dest_file_path: &str,
) -> Result<Option<PathBuf>, String> {
    if source_file_path != dest_file_path {
        return Ok(None);
    }

    let source_path = Path::new(source_file_path);
    let extension = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("tmp");
    let backup_path = std::env::temp_dir().join(format!(
        "lap-edit-metadata-{}.{}",
        Uuid::new_v4(),
        extension
    ));

    fs::copy(source_path, &backup_path)
        .map_err(|e| format!("Failed to create metadata backup: {}", e))?;

    Ok(Some(backup_path))
}

fn cleanup_metadata_backup(path: &Option<PathBuf>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

/// Copies metadata from source to destination.
/// Prefers little_exif for JPEGs and falls back to kamadak-exif for RAW formats.
fn copy_metadata_to_output(source_path: &Path, dest_path: &Path) -> Result<(), String> {
    let source_path_buf = source_path.to_path_buf();

    // Check file type to detect RAW formats
    let file_type =
        crate::t_utils::get_file_type(source_path.to_str().unwrap_or_default()).unwrap_or(0);
    let is_raw = file_type == 3;

    let mut little_exif_worked = false;
    let mut little_exif_error = String::new();

    if !is_raw {
        // Use little_exif for standard formats (JPEG/WebP).
        // Wrapped in catch_unwind as little_exif can panic on malformed data.
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            LittleExifMetadata::new_from_path(&source_path_buf)
        }));

        match result {
            Ok(Ok(mut metadata)) => {
                sanitize_edit_output_metadata(&mut metadata);
                if let Err(e) = metadata.write_to_file(dest_path) {
                    little_exif_error = format!("little_exif write failed: {}", e);
                } else {
                    little_exif_worked = true;
                }
            }
            Ok(Err(e)) => {
                little_exif_error = format!("little_exif read failed: {}", e);
            }
            Err(_) => {
                little_exif_error = "little_exif panicked".to_string();
            }
        }
    }

    if little_exif_worked {
        return Ok(());
    }

    // Fallback: use kamadak-exif which has broader RAW support
    match copy_metadata_from_raw_to_jpeg(source_path, dest_path) {
        Ok(()) => Ok(()),
        Err(raw_error) => {
            if is_raw {
                Err(format!("RAW metadata extraction failed: {}", raw_error))
            } else {
                Err(format!(
                    "Metadata copy failed. little_exif: {}; kamadak: {}",
                    little_exif_error, raw_error
                ))
            }
        }
    }
}

/// Removes tags that shouldn't be copied to the edited output (like original orientation and dimensions).
fn sanitize_edit_output_metadata(metadata: &mut LittleExifMetadata) {
    metadata.remove_tag_by_hex_group(0x0112, ExifTagGroup::GENERIC); // Orientation
    metadata.remove_tag_by_hex_group(0x0100, ExifTagGroup::GENERIC); // ImageWidth
    metadata.remove_tag_by_hex_group(0x0101, ExifTagGroup::GENERIC); // ImageLength
    metadata.remove_tag_by_hex_group(0xA002, ExifTagGroup::EXIF); // PixelXDimension
    metadata.remove_tag_by_hex_group(0xA003, ExifTagGroup::EXIF); // PixelYDimension
    metadata.remove_tag_by_hex_group(0x0201, ExifTagGroup::GENERIC); // JPEGInterchangeFormat
    metadata.remove_tag_by_hex_group(0x0202, ExifTagGroup::GENERIC); // JPEGInterchangeFormatLength
}

/// Filter for EXIF fields to copy.
/// We mainly copy PRIMARY IFD and exclude pointers or hardware-specific tags that
/// might be invalidated by the image edit (like StripOffsets or Orientation).
fn should_copy_exif_field(field: &exif::Field) -> bool {
    if field.ifd_num != In::PRIMARY {
        return false;
    }

    !matches!(
        field.tag,
        Tag::ExifIFDPointer
            | Tag::GPSInfoIFDPointer
            | Tag::InteropIFDPointer
            | Tag::StripOffsets
            | Tag::StripByteCounts
            | Tag::TileOffsets
            | Tag::TileByteCounts
            | Tag::JPEGInterchangeFormat
            | Tag::JPEGInterchangeFormatLength
            | Tag::Orientation
            | Tag::ImageWidth
            | Tag::ImageLength
            | Tag::PixelXDimension
            | Tag::PixelYDimension
    )
}

/// Extracts EXIF from a (potentially RAW) source and injects it into a JPEG destination.
/// Includes a three-pass reduction logic to ensure metadata fits within the 64KB JPEG segment limit.
fn copy_metadata_from_raw_to_jpeg(source_path: &Path, dest_path: &Path) -> Result<(), String> {
    let file = File::open(source_path)
        .map_err(|e| format!("Failed to open source metadata file: {}", e))?;
    let mut reader = BufReader::new(file);

    // Try read_from_container first (handles JPEG/TIFF/RAW-TIFF)
    let exif = match Reader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        Err(_) => {
            // Fallback: try raw TIFF read (some RAWs are just TIFF structures)
            let data = fs::read(source_path).map_err(|e| e.to_string())?;
            match Reader::new().read_raw(data) {
                Ok(exif) => exif,
                Err(_) => return Ok(()), // Truly no metadata found or unreadable, skip
            }
        }
    };

    // Helper to attempt encoding a set of fields and check if it fits in 64KB
    let encode_and_check = |fields: Vec<&exif::Field>| -> Option<Vec<u8>> {
        let mut writer = exif::experimental::Writer::new();
        for field in fields {
            writer.push_field(field);
        }
        let mut tiff_cursor = Cursor::new(Vec::new());
        if writer.write(&mut tiff_cursor, exif.little_endian()).is_ok() {
            let data = tiff_cursor.into_inner();
            if data.len() <= 65527 {
                // JPEG APP1 max is 65535, minus 8 bytes header
                return Some(data);
            }
        }
        None
    };

    // Pass 1: Attempt to copy all standard fields
    let initial_fields: Vec<&exif::Field> = exif
        .fields()
        .filter(|f| should_copy_exif_field(f))
        .collect();
    let mut exif_data = encode_and_check(initial_fields);

    // Pass 2: If too large, strip typically large vendor blocks (MakerNote, UserComment)
    if exif_data.is_none() {
        let reduced_fields: Vec<&exif::Field> = exif
            .fields()
            .filter(|f| should_copy_exif_field(f))
            .filter(|f| !matches!(f.tag, Tag::MakerNote | Tag::UserComment))
            .collect();
        exif_data = encode_and_check(reduced_fields);
    }

    // Pass 3: If still too large, keep only the most essential photography and GPS tags
    if exif_data.is_none() {
        let essential_fields: Vec<&exif::Field> = exif
            .fields()
            .filter(|f| should_copy_exif_field(f))
            .filter(|f| {
                matches!(
                    f.tag,
                    Tag::Make
                        | Tag::Model
                        | Tag::DateTimeOriginal
                        | Tag::DateTimeDigitized
                        | Tag::ExposureTime
                        | Tag::FNumber
                        | Tag::PhotographicSensitivity
                        | Tag::FocalLength
                        | Tag::LensMake
                        | Tag::LensModel
                        | Tag::ExposureBiasValue
                        | Tag::GPSLatitudeRef
                        | Tag::GPSLatitude
                        | Tag::GPSLongitudeRef
                        | Tag::GPSLongitude
                        | Tag::GPSAltitudeRef
                        | Tag::GPSAltitude
                )
            })
            .collect();
        exif_data = encode_and_check(essential_fields);
    }

    if let Some(data) = exif_data {
        write_jpeg_exif_block(dest_path, &data)
    } else {
        eprintln!("EXIF metadata still too large even after stripping, skipping");
        Ok(())
    }
}

/// Manually injects a TIFF-formatted EXIF block into a JPEG file's APP1 segment.
/// This is used when high-level libraries (like little_exif) fail or are not applicable.
fn write_jpeg_exif_block(dest_path: &Path, exif_tiff_data: &[u8]) -> Result<(), String> {
    let mut file_buffer =
        fs::read(dest_path).map_err(|e| format!("Failed to read destination JPEG: {}", e))?;

    // Clear existing metadata using little_exif if possible
    let _ = LittleExifMetadata::clear_metadata(&mut file_buffer, FileExtension::JPEG);

    if file_buffer.len() < 2 || file_buffer[0] != 0xFF || file_buffer[1] != 0xD8 {
        return Err("Destination file is not a valid JPEG".to_string());
    }

    // Prepare APP1 segment: FF E1 + Length + "Exif\0\0" + TIFF data
    let app1_length = (2 + 6 + exif_tiff_data.len()) as u16;
    let mut app1_segment = Vec::with_capacity(2 + 2 + 6 + exif_tiff_data.len());
    app1_segment.extend_from_slice(&[0xFF, 0xE1]);
    app1_segment.extend_from_slice(&app1_length.to_be_bytes());
    app1_segment.extend_from_slice(b"Exif\0\0");
    app1_segment.extend_from_slice(exif_tiff_data);

    // Reconstruct file: FF D8 + New APP1 + Remainder of file
    let mut output = Vec::with_capacity(file_buffer.len() + app1_segment.len());
    output.extend_from_slice(&file_buffer[..2]);
    output.extend_from_slice(&app1_segment);
    output.extend_from_slice(&file_buffer[2..]);

    fs::write(dest_path, output).map_err(|e| format!("Failed to write destination JPEG: {}", e))
}

/// copy an image to clipboard
pub fn copy_image_to_clipboard(img: DynamicImage) -> bool {
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8();
    let bytes = rgba.into_raw();

    if let Ok(mut clipboard) = Clipboard::new() {
        let image_data = arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(bytes),
        };
        return clipboard.set_image(image_data).is_ok();
    }
    false
}

pub(crate) fn is_heic_path(file_path: &str) -> bool {
    matches!(
        t_utils::get_file_extension(file_path)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "heic" | "heif" | "hif"
    )
}

fn is_avif_path(file_path: &str) -> bool {
    matches!(
        t_utils::get_file_extension(file_path)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "avif"
    )
}

pub(crate) fn is_ffmpeg_backed_image_path(file_path: &str) -> bool {
    t_utils::get_file_extension(file_path).is_some_and(|extension| {
        crate::t_common::FFMPEG_BACKED_IMGS
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&extension))
    })
}

fn is_preview_only_document(file_path: &str) -> bool {
    crate::t_affinity::is_affinity_path(file_path)
        || Path::new(file_path)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb"))
}

/// Longest side of a PSD/PSB composite preview shown in the viewer (never upscaled).
const PSD_VIEWER_MAX_DIMENSION: u32 = 16384;

/// Decodes the merged image of a PSD/PSB and encodes it as JPEG (no upscaling).
fn psd_composite_jpeg(file_path: &str, max_dimension: u32) -> Result<Vec<u8>, String> {
    let rgb = crate::t_psd::decode_composite(file_path)?;
    resize_rgb_image_to_jpeg(rgb, max_dimension)
}

/// Recently produced PSD/PSB viewer previews, so reopening (or opening after a
/// background preload) does not decode a multi-hundred-megapixel composite again.
static PSD_PREVIEW_CACHE: Lazy<std::sync::Mutex<Vec<(String, u64, u64, std::sync::Arc<Vec<u8>>)>>> =
    Lazy::new(|| std::sync::Mutex::new(Vec::new()));
const PSD_PREVIEW_CACHE_ENTRIES: usize = 10;

fn psd_cache_key(file_path: &str) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(file_path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((mtime, meta.len()))
}

/// Decoded RGB pictures of recently shown big photos (PSD composites, PNG, BMP, TIFF). The hybrid
/// mode asks for a fit preview and then the full-resolution image of the same file; sharing one
/// decode makes the second request cost only the resize/encode. Oldest entries go first.
type ViewerRgbEntry = (String, u64, u64, std::sync::Arc<RgbImage>);
static VIEWER_RGB_CACHE: Lazy<std::sync::Mutex<Vec<ViewerRgbEntry>>> =
    Lazy::new(|| std::sync::Mutex::new(Vec::new()));
static VIEWER_RGB_INFLIGHT: Lazy<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<()>>>>,
> = Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
/// Total size of the cached RGB pictures (about 750 MB).
const VIEWER_RGB_CACHE_MAX_BYTES: u64 = 750 * 1024 * 1024;

fn shared_rgb<F>(file_path: &str, tag: &str, decode: F) -> Result<std::sync::Arc<RgbImage>, String>
where
    F: FnOnce() -> Result<RgbImage, String>,
{
    let key = psd_cache_key(file_path);
    let id = format!("{}\u{0}{}", file_path, tag);
    let lookup = || -> Option<std::sync::Arc<RgbImage>> {
        let (mtime, len) = key?;
        let mut guard = VIEWER_RGB_CACHE.lock().ok()?;
        let pos = guard
            .iter()
            .position(|(p, m, l, _)| *p == id && *m == mtime && *l == len)?;
        let entry = guard.remove(pos);
        let image = entry.3.clone();
        guard.push(entry);
        Some(image)
    };
    if let Some(image) = lookup() {
        return Ok(image);
    }
    // One decode per file at a time: a concurrent request for the same file waits and then finds it.
    let lock = {
        let mut map = VIEWER_RGB_INFLIGHT
            .lock()
            .map_err(|_| "decode lock poisoned".to_string())?;
        map.entry(id.clone())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().map_err(|_| "decode lock poisoned".to_string())?;
    let result = match lookup() {
        Some(image) => Ok(image),
        None => decode().map(std::sync::Arc::new).map(|image| {
            if let Some((mtime, len)) = key {
                let bytes = image.width() as u64 * image.height() as u64 * 3;
                if bytes <= VIEWER_RGB_CACHE_MAX_BYTES {
                    if let Ok(mut cache) = VIEWER_RGB_CACHE.lock() {
                        cache.retain(|(p, _, _, _)| *p != id);
                        cache.push((id.clone(), mtime, len, image.clone()));
                        let mut total: u64 = cache
                            .iter()
                            .map(|(_, _, _, img)| img.width() as u64 * img.height() as u64 * 3)
                            .sum();
                        while total > VIEWER_RGB_CACHE_MAX_BYTES && cache.len() > 1 {
                            let (_, _, _, removed) = cache.remove(0);
                            total -= removed.width() as u64 * removed.height() as u64 * 3;
                        }
                    }
                }
            }
            image
        }),
    };
    if let Ok(mut map) = VIEWER_RGB_INFLIGHT.lock() {
        map.remove(&id);
    }
    result
}

fn psd_decode_shared(file_path: &str) -> Result<std::sync::Arc<RgbImage>, String> {
    shared_rgb(file_path, "psd", || crate::t_psd::decode_composite(file_path))
}

fn psd_viewer_full_jpeg(file_path: &str) -> Result<Vec<u8>, String> {
    let rgb = psd_decode_shared(file_path)?;
    resize_rgb_image_to_jpeg((*rgb).clone(), PSD_VIEWER_MAX_DIMENSION)
}

/// Full-resolution viewer preview of a PSD/PSB; None when the native reader cannot handle the file.
async fn psd_viewer_preview(file_path: &str) -> Option<Vec<u8>> {
    cached_viewer_jpeg(file_path, "psd".to_string(), psd_viewer_full_jpeg).await
}

/// PNGs at or above this many pixels are served to the viewer as a backend-made JPEG.
const PNG_VIEWER_MIN_PIXELS: u64 = 12_000_000;
/// JPEG quality for those previews (4:4:4, so only barely distinguishable from the PNG).
const PNG_VIEWER_JPEG_QUALITY: u8 = 97;
/// JPEG quality of the screen-sized "fit" previews.
const FIT_PREVIEW_JPEG_QUALITY: u8 = 92;

/// Colour shown through transparent PNG pixels in the JPEG preview (0xRRGGBB); set from the viewer.
static PNG_ALPHA_BG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x5a5a5a);

pub fn set_png_alpha_background(rgb: u32) {
    PNG_ALPHA_BG.store(rgb & 0xffffff, std::sync::atomic::Ordering::Relaxed);
}

/// True for big sRGB-like PNGs (with or without transparency), where a JPEG preview is a good stand-in.
/// PNGs with a non-sRGB ICC profile keep the original bytes.
fn is_large_opaque_png(file_path: &str) -> bool {
    use std::io::Read;
    let is_png = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
    if !is_png {
        return false;
    }
    let Ok(file) = std::fs::File::open(file_path) else {
        return false;
    };
    let mut buf = Vec::with_capacity(65536);
    if file.take(65536).read_to_end(&mut buf).is_err() || buf.len() < 33 {
        return false;
    }
    if &buf[0..8] != b"\x89PNG\r\n\x1a\n" || &buf[12..16] != b"IHDR" {
        return false;
    }
    let width = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]) as u64;
    let height = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]) as u64;
    let colour_type = buf[25];
    let _ = colour_type;
    if width * height < PNG_VIEWER_MIN_PIXELS {
        return false;
    }
    // Walk the chunks before the image data.
    let mut pos = 33usize;
    while pos + 8 <= buf.len() {
        let len = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
        let kind = &buf[pos + 4..pos + 8];
        if kind == b"IDAT" {
            break;
        }
        if kind == b"iCCP" {
            let end = (pos + 8 + len).min(buf.len());
            let body = &buf[pos + 8..end];
            let name_len = body.iter().position(|b| *b == 0).unwrap_or(body.len());
            let name = String::from_utf8_lossy(&body[..name_len]).to_ascii_lowercase();
            if !name.contains("srgb") {
                return false;
            }
        }
        pos = pos.saturating_add(12).saturating_add(len);
    }
    true
}

/// Big uncompressed 8/24-bit BMP (no alpha, no RLE, sRGB or no colour-space info): decoded by the
/// backend like a big PNG, because WebView2 decodes a 100 MB bitmap slowly.
fn is_large_bmp(file_path: &str) -> bool {
    use std::io::Read;
    let is_bmp = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bmp"));
    if !is_bmp {
        return false;
    }
    let Ok(file) = std::fs::File::open(file_path) else {
        return false;
    };
    let mut buf = Vec::with_capacity(160);
    if file.take(160).read_to_end(&mut buf).is_err() || buf.len() < 54 || &buf[0..2] != b"BM" {
        return false;
    }
    let u32_at = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    let header_size = u32_at(14);
    if header_size < 40 {
        return false;
    }
    let width = (u32_at(18) as i32).unsigned_abs() as u64;
    let height = (u32_at(22) as i32).unsigned_abs() as u64;
    let bpp = u16::from_le_bytes([buf[28], buf[29]]);
    let compression = u32_at(30);
    if width * height < PNG_VIEWER_MIN_PIXELS || compression != 0 || (bpp != 24 && bpp != 8) {
        return false;
    }
    if header_size >= 108 && buf.len() >= 74 {
        // bV4/bV5 colour space type: only sRGB or "Windows default" are shown as sRGB.
        let cs_type = u32_at(70);
        if cs_type != 0x7352_4742 && cs_type != 0x5769_6E20 {
            return false;
        }
    }
    true
}

/// PNG / BMP decoded to RGB (transparency flattened), shared between the fit and full previews.
fn png_decode_shared(file_path: &str) -> Result<std::sync::Arc<RgbImage>, String> {
    let tag = format!("png-{}", PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed));
    shared_rgb(file_path, &tag, || {
        let mut reader = ImageReader::open(file_path)
            .map_err(|e| format!("Failed to open image: {}", e))?
            .with_guessed_format()
            .map_err(|e| format!("Failed to read image: {}", e))?;
        reader.no_limits();
        let decoded = reader
            .decode()
            .map_err(|e| format!("Failed to decode image: {}", e))?;
        flatten_for_viewer(decoded)
    })
}

fn png_viewer_jpeg(file_path: &str) -> Result<Vec<u8>, String> {
    let rgb = png_decode_shared(file_path)?;
    crate::t_jpeg::encode_rgb8_ex(&rgb, PNG_VIEWER_JPEG_QUALITY, true)
}

/// Converts to RGB8; transparent pixels are blended over the viewer's transparency colour.
fn flatten_for_viewer(decoded: DynamicImage) -> Result<RgbImage, String> {
    let rgb = if decoded.color().has_alpha() {
        let bg = PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed);
        let (br, bgc, bb) = (((bg >> 16) & 255) as u32, ((bg >> 8) & 255) as u32, (bg & 255) as u32);
        let rgba = decoded.into_rgba8();
        let (w, h) = rgba.dimensions();
        let mut out = vec![0u8; (w as usize) * (h as usize) * 3];
        for (src, dst) in rgba.as_raw().chunks_exact(4).zip(out.chunks_exact_mut(3)) {
            let a = src[3] as u32;
            dst[0] = ((src[0] as u32 * a + br * (255 - a) + 127) / 255) as u8;
            dst[1] = ((src[1] as u32 * a + bgc * (255 - a) + 127) / 255) as u8;
            dst[2] = ((src[2] as u32 * a + bb * (255 - a) + 127) / 255) as u8;
        }
        RgbImage::from_raw(w, h, out).ok_or_else(|| "Failed to build RGB image".to_string())?
    } else {
        decoded.into_rgb8()
    };
    Ok(rgb)
}

/// Native full-resolution decode of a TIFF (any size) as a high-quality JPEG; fails for
/// files the `image` crate cannot read (e.g. CMYK), in which case the old path is used.
fn tiff_decode_rgb(file_path: &str) -> Result<RgbImage, String> {
    use image::ImageDecoder;
    let mut reader = ImageReader::open(file_path)
        .map_err(|e| format!("Failed to open TIFF: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("Failed to read TIFF: {}", e))?;
    reader.no_limits();
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("Failed to open TIFF decoder: {}", e))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Failed to decode TIFF: {}", e))?;
    decoded.apply_orientation(orientation);
    flatten_for_viewer(decoded)
}

fn tiff_decode_shared(file_path: &str) -> Result<std::sync::Arc<RgbImage>, String> {
    let tag = format!("tiff-{}", PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed));
    shared_rgb(file_path, &tag, || tiff_decode_rgb(file_path))
}

fn tiff_viewer_jpeg(file_path: &str) -> Result<Vec<u8>, String> {
    let rgb = tiff_decode_shared(file_path)?;
    crate::t_jpeg::encode_rgb8_ex(&rgb, PNG_VIEWER_JPEG_QUALITY, true)
}

fn tiff_fit_jpeg(file_path: &str, max_edge: u32, opts: FitOpts) -> Result<Vec<u8>, String> {
    let rgb = tiff_decode_shared(file_path)?;
    fit_resize_encode((*rgb).clone(), max_edge, opts)
}

fn psd_fit_jpeg(file_path: &str, max_edge: u32, opts: FitOpts) -> Result<Vec<u8>, String> {
    let rgb = psd_decode_shared(file_path)?;
    fit_resize_encode((*rgb).clone(), max_edge, opts)
}

/// True when the image header says the picture has at least the hybrid-mode pixel count.
fn is_big_by_header(file_path: &str) -> bool {
    imagesize::size(file_path)
        .map(|s| (s.width as u64) * (s.height as u64) >= PNG_VIEWER_MIN_PIXELS)
        .unwrap_or(false)
}

async fn tiff_viewer_preview(file_path: &str) -> Option<Vec<u8>> {
    cached_viewer_jpeg(file_path, format!("tiff-{}", PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed)), tiff_viewer_jpeg).await
}

// ---- Hybrid mode: screen-sized "fit" previews of big JPEG / PNG files --------------------

/// Big JPEGs are only handled when they carry no ICC profile or an sRGB one
/// (the re-encoded preview carries no profile, so other profiles would shift colours).
fn jpeg_fit_eligible(file_path: &str) -> bool {
    use std::io::Read;
    let Ok(size) = imagesize::size(file_path) else {
        return false;
    };
    if (size.width as u64) * (size.height as u64) < PNG_VIEWER_MIN_PIXELS {
        return false;
    }
    let Ok(file) = std::fs::File::open(file_path) else {
        return false;
    };
    let mut buf = Vec::with_capacity(262144);
    if file.take(262144).read_to_end(&mut buf).is_err() || buf.len() < 4 || buf[0] != 0xFF || buf[1] != 0xD8 {
        return false;
    }
    let mut pos = 2usize;
    let mut icc = Vec::new();
    while pos + 4 <= buf.len() {
        if buf[pos] != 0xFF {
            break;
        }
        let marker = buf[pos + 1];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
        let end = (pos + 2 + len).min(buf.len());
        if marker == 0xE2 && buf[pos + 4..end].starts_with(b"ICC_PROFILE\0") {
            icc.extend_from_slice(&buf[pos + 4..end]);
        }
        pos = pos + 2 + len;
    }
    icc.is_empty() || icc.windows(4).any(|w| w == b"sRGB")
}

/// Lanczos-resizes to `max_edge` on the long side (never upscales) and encodes a 4:4:4 JPEG.
/// Options of a hybrid-mode fit preview: sharpening (0-100) and resize filter
/// (0 Lanczos3, 1 Catmull-Rom, 2 Mitchell).
#[derive(Clone, Copy, Debug)]
pub struct FitOpts {
    pub sharp: u32,
    pub filter: u32,
    /// Keep the finished preview on disk (otherwise only in memory).
    pub disk: bool,
}

/// Mild unsharp mask (3x1 / 1x3 binomial blur) to bring a Lanczos-downscaled preview closer to the
/// crispness of a GPU downscale. `amount_pct` 0 disables it; 100 adds the full detail difference once.
fn unsharp_rgb8(raw: &mut [u8], w: usize, h: usize, amount_pct: u32) {
    if amount_pct == 0 || w < 3 || h < 3 || raw.len() != w * h * 3 {
        return;
    }
    let stride = w * 3;
    // Horizontal pass (kept at x4 scale to avoid rounding): [1 2 1].
    let mut horiz = vec![0u16; raw.len()];
    for y in 0..h {
        let row = &raw[y * stride..(y + 1) * stride];
        let out = &mut horiz[y * stride..(y + 1) * stride];
        for x in 0..w {
            let l = if x == 0 { 0 } else { x - 1 };
            let r = if x + 1 == w { x } else { x + 1 };
            for c in 0..3 {
                out[x * 3 + c] =
                    row[l * 3 + c] as u16 + 2 * row[x * 3 + c] as u16 + row[r * 3 + c] as u16;
            }
        }
    }
    let amount = amount_pct as i32;
    // Vertical pass gives blur x16; combine with the source in one go.
    let mut out_rows = vec![0u8; raw.len()];
    for y in 0..h {
        let u = if y == 0 { 0 } else { y - 1 };
        let d = if y + 1 == h { y } else { y + 1 };
        for i in 0..stride {
            let blur16 = horiz[u * stride + i] as i32 + 2 * horiz[y * stride + i] as i32 + horiz[d * stride + i] as i32;
            let src = raw[y * stride + i] as i32;
            // detail = src - blur; result = src + amount% * detail  (blur16 is blur * 16)
            let detail16 = src * 16 - blur16;
            let v = src + (detail16 * amount) / (16 * 100);
            out_rows[y * stride + i] = v.clamp(0, 255) as u8;
        }
    }
    raw.copy_from_slice(&out_rows);
}

fn fit_resize_encode(rgb: RgbImage, max_edge: u32, opts: FitOpts) -> Result<Vec<u8>, String> {
    let (src_w, src_h) = rgb.dimensions();
    let long = src_w.max(src_h);
    if long <= max_edge {
        return crate::t_jpeg::encode_rgb8_ex(&rgb, FIT_PREVIEW_JPEG_QUALITY, true);
    }
    let ratio = max_edge as f64 / long as f64;
    let dst_w = ((src_w as f64 * ratio).round() as u32).max(1);
    let dst_h = ((src_h as f64 * ratio).round() as u32).max(1);
    let src_image = fir::images::Image::from_vec_u8(src_w, src_h, rgb.into_raw(), fir::PixelType::U8x3)
        .map_err(|e| format!("Failed to prepare image for resize: {}", e))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x3);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(match opts.filter {
            1 => fir::FilterType::CatmullRom,
            2 => fir::FilterType::Mitchell,
            _ => fir::FilterType::Lanczos3,
        }));
    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| format!("Failed to resize preview: {}", e))?;
    let mut buffer = dst_image.into_vec();
    unsharp_rgb8(&mut buffer, dst_w as usize, dst_h as usize, opts.sharp);
    let resized = RgbImage::from_raw(dst_w, dst_h, buffer)
        .ok_or_else(|| "Failed to build resized image".to_string())?;
    crate::t_jpeg::encode_rgb8_ex(&resized, FIT_PREVIEW_JPEG_QUALITY, true)
}

fn jpeg_fit_jpeg(file_path: &str, max_edge: u32, opts: FitOpts) -> Result<Vec<u8>, String> {
    let size = imagesize::size(file_path).map_err(|e| format!("Failed to read JPEG size: {}", e))?;
    let (w, h) = (size.width as u32, size.height as u32);
    let ratio = (max_edge as f64 / w.max(h).max(1) as f64).min(1.0);
    // The decoder picks the largest DCT downscale that still yields at least this size.
    let target_w = ((w as f64 * ratio).ceil() as u32).max(1);
    let target_h = ((h as f64 * ratio).ceil() as u32).max(1);
    let (pixels, pw, ph) = crate::t_jpeg::decode_rgb8_scaled(file_path, target_w, target_h)?;
    let rgb = RgbImage::from_raw(pw, ph, pixels).ok_or_else(|| "Bad JPEG pixel buffer".to_string())?;
    let oriented = apply_orientation(DynamicImage::ImageRgb8(rgb), get_image_orientation(file_path));
    fit_resize_encode(oriented.into_rgb8(), max_edge, opts)
}

fn png_fit_jpeg(file_path: &str, max_edge: u32, opts: FitOpts) -> Result<Vec<u8>, String> {
    let rgb = png_decode_shared(file_path)?;
    fit_resize_encode((*rgb).clone(), max_edge, opts)
}

/// Screen-sized preview (long side `max_edge`) of a big JPEG or PNG for the viewer's hybrid
/// mode. None means "not applicable": the caller then serves the original file.
pub async fn get_fit_preview(file_path: &str, max_edge: u32, opts: FitOpts) -> Option<Vec<u8>> {
    if !(512..=16384).contains(&max_edge) {
        return None;
    }
    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    let bg = PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed);
    let opts = FitOpts { sharp: opts.sharp.min(100), filter: opts.filter.min(2), disk: opts.disk };
    // The "fit-" prefix means the preview is also kept on disk, "fitm-" memory only.
    let tag = format!(
        "{}-{}-{}-{}-{}",
        if opts.disk { "fit" } else { "fitm" },
        max_edge,
        bg,
        opts.sharp,
        opts.filter
    );
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" => {
            if !jpeg_fit_eligible(file_path) {
                return None;
            }
            cached_viewer_jpeg(file_path, tag, move |p| jpeg_fit_jpeg(p, max_edge, opts)).await
        }
        "png" => {
            if !is_large_opaque_png(file_path) {
                return None;
            }
            cached_viewer_jpeg(file_path, tag, move |p| png_fit_jpeg(p, max_edge, opts)).await
        }
        "bmp" => {
            if !is_large_bmp(file_path) {
                return None;
            }
            cached_viewer_jpeg(file_path, tag, move |p| png_fit_jpeg(p, max_edge, opts)).await
        }
        "tif" | "tiff" => {
            if !is_big_by_header(file_path) {
                return None;
            }
            cached_viewer_jpeg(file_path, tag, move |p| tiff_fit_jpeg(p, max_edge, opts)).await
        }
        "psd" | "psb" => {
            if !is_big_by_header(file_path) {
                return None;
            }
            cached_viewer_jpeg(file_path, tag, move |p| psd_fit_jpeg(p, max_edge, opts)).await
        }
        _ => None,
    }
}

/// Viewer preview of a big PNG as a full-resolution JPEG; None when not applicable or on failure.
async fn png_viewer_preview(file_path: &str) -> Option<Vec<u8>> {
    if !is_large_opaque_png(file_path) && !is_large_bmp(file_path) {
        return None;
    }
    cached_viewer_jpeg(file_path, format!("png-{}", PNG_ALPHA_BG.load(std::sync::atomic::Ordering::Relaxed)), png_viewer_jpeg).await
}

const FIT_DISK_CACHE_MAX_FILES: usize = 400;

fn fit_disk_cache_dir() -> Option<std::path::PathBuf> {
    crate::t_config::get_app_data_dir().ok().map(|d| d.join("fit-cache"))
}

fn fit_disk_cache_path(file_path: &str, tag: &str, mtime: u64, len: u64) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_path.hash(&mut hasher);
    tag.hash(&mut hasher);
    mtime.hash(&mut hasher);
    len.hash(&mut hasher);
    Some(fit_disk_cache_dir()?.join(format!("{:016x}.jpg", hasher.finish())))
}

/// Keeps the disk cache bounded: removes the least recently written files beyond the limit.
fn prune_fit_disk_cache() {
    let Some(dir) = fit_disk_cache_dir() else {
        return;
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if files.len() <= FIT_DISK_CACHE_MAX_FILES {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    let excess = files.len() - FIT_DISK_CACHE_MAX_FILES;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

async fn cached_viewer_jpeg<F>(file_path: &str, tag: String, produce: F) -> Option<Vec<u8>>
where
    F: FnOnce(&str) -> Result<Vec<u8>, String> + Send + 'static,
{
    // The tag (e.g. the transparency colour) is part of the cache identity.
    let cache_path = format!("{}\u{0}{}", file_path, tag);
    let file_path_orig = file_path;
    let file_path = cache_path.as_str();
    let key = psd_cache_key(file_path_orig);
    if let Some((mtime, len)) = key {
        if let Ok(cache) = PSD_PREVIEW_CACHE.lock() {
            if let Some(entry) = cache
                .iter()
                .find(|(p, m, l, _)| p == file_path && *m == mtime && *l == len)
            {
                return Some(entry.3.as_ref().clone());
            }
        }
    }
    // Screen-sized previews are small, so they are also kept on disk: reopening a big photo
    // later (even after a restart) skips the slow decode.
    let disk_path = if tag.starts_with("fit-") {
        key.and_then(|(mtime, len)| fit_disk_cache_path(file_path_orig, &tag, mtime, len))
    } else {
        None
    };
    if let Some(p) = &disk_path {
        if let Ok(bytes) = tokio::fs::read(p).await {
            if bytes.len() > 1024 {
                return Some(bytes);
            }
        }
    }
    let path = file_path_orig.to_string();
    let data = tauri::async_runtime::spawn_blocking(move || produce(&path))
    .await
    .ok()?
    .ok()?;
    if let Some(p) = disk_path {
        if let Some(dir) = p.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let tmp = p.with_extension("tmp");
        if tokio::fs::write(&tmp, &data).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, &p).await;
        }
        prune_fit_disk_cache();
    }
    if let Some((mtime, len)) = key {
        if let Ok(mut cache) = PSD_PREVIEW_CACHE.lock() {
            cache.retain(|(p, _, _, _)| p != file_path);
            cache.push((file_path.to_string(), mtime, len, std::sync::Arc::new(data.clone())));
            while cache.len() > PSD_PREVIEW_CACHE_ENTRIES {
                cache.remove(0);
            }
        }
    }
    Some(data)
}

fn should_generate_preview_for_file(file_path: &str, file_type: i64) -> bool {
    file_type == 3
        || crate::t_libraw::is_tiff_path(file_path)
        || t_jxl::is_jxl_path(file_path)
        || crate::t_affinity::is_affinity_path(file_path)
        || is_heic_path(file_path)
        || is_ffmpeg_backed_image_path(file_path)
        || is_avif_path(file_path)
}

pub(crate) async fn get_editable_preview_bytes(file_path: &str) -> Result<Option<Vec<u8>>, String> {
    if is_preview_only_document(file_path) {
        return Ok(None);
    }
    get_generated_preview_bytes(file_path).await
}

async fn get_generated_preview_bytes(file_path: &str) -> Result<Option<Vec<u8>>, String> {
    t_utils::require_local_content(Path::new(file_path)).map_err(|e| e.to_string())?;
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);

    if file_type == 3 {
        return get_raw_preview_image(file_path, RawDisplayOptions::rendered_bright());
    }

    if crate::t_affinity::is_affinity_path(file_path) {
        let path = file_path.to_string();
        return tauri::async_runtime::spawn_blocking(move || crate::t_affinity::preview_png(&path))
            .await
            .map_err(|e| e.to_string());
    }

    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_preview_image(file_path, 4096);
    }

    if crate::t_libraw::is_tiff_path(file_path) {
        return match get_raw_preview_image(file_path, RawDisplayOptions::rendered_bright()) {
            Ok(Some(data)) => Ok(Some(data)),
            _ => {
                #[cfg(target_os = "macos")]
                {
                    get_thumbnail_with_sips(file_path, 4096)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Ok(None)
                }
            }
        };
    }

    if is_heic_path(file_path) {
        return crate::t_heif::get_heif_preview(
            file_path,
            get_image_orientation(file_path),
            4096,
        );
    }

    if crate::t_psd::is_psd_path(file_path) {
        if let Some(data) = psd_viewer_preview(file_path).await {
            return Ok(Some(data));
        }
    }

    if is_ffmpeg_backed_image_path(file_path) {
        return crate::t_video::get_video_thumbnail(file_path, 4096, None, None).await;
    }

    if is_avif_path(file_path) {
        return get_image_thumbnail(file_path, get_image_orientation(file_path), 4096);
    }

    Ok(None)
}

async fn clipboard_preview_png(file_paths: &[String]) -> Option<Vec<u8>> {
    for file_path in file_paths {
        if !t_utils::content_readable(file_path) {
            continue;
        }
        let file_type = t_utils::get_file_type(file_path).unwrap_or(0);
        if file_type != 1 && file_type != 3 {
            continue;
        }
        let img = if should_generate_preview_for_file(file_path, file_type) {
            let Some(preview) = get_generated_preview_bytes(file_path).await.ok().flatten() else {
                continue;
            };
            let Ok(img) = image::load_from_memory(&preview) else {
                continue;
            };
            img
        } else {
            let Ok(img) = image::open(Path::new(file_path)) else {
                continue;
            };
            img
        };
        let mut png = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut png, image::ImageFormat::Png).is_ok() {
            return Some(png.into_inner());
        }
    }
    None
}

pub async fn copy_files_to_clipboard(
    app_handle: &tauri::AppHandle,
    file_paths: Vec<String>,
    files_only: bool,
) -> Result<usize, String> {
    let file_paths = file_paths
        .into_iter()
        // The UI limits copying to 10 content items. RAW+JPEG pairs expand
        // one item into two physical files, so retain both components.
        .take(20)
        .collect::<Vec<_>>();
    if file_paths.is_empty() {
        return Err("No valid files to copy".to_string());
    }
    // Do not silently copy only part of a RAW pair or multi-file selection.
    if let Some(path) = file_paths.iter().find(|path| !Path::new(path).is_file()) {
        return Err(format!("File is unavailable: {}", path));
    }
    // A single-image representation lets paste targets pick pixels instead of
    // the file list. Copy a complete RAW pair as files only.
    let preview = if files_only { None } else { clipboard_preview_png(&file_paths).await };
    crate::t_pasteboard::copy_files_and_image(app_handle, &file_paths, preview.as_deref()).await?;
    Ok(file_paths.len())
}

/// copy an edited image to clipboard
pub async fn copy_edited_image_to_clipboard(params: EditParams) -> bool {
    if let Ok(img) = get_edited_image(&params).await {
        return copy_image_to_clipboard(img);
    }
    false
}

/// Rotate an image by an arbitrary angle (degrees), producing an axis-aligned
/// bounding-box output with transparent corners, using bilinear interpolation.
/// Takes `img` by value and uses `into_rgba8` so an already-RGBA source reuses
/// its buffer instead of allocating a second full-image copy.
pub(crate) fn rotate_arbitrary(img: DynamicImage, angle_deg: f32) -> DynamicImage {
    let angle = angle_deg.to_radians();
    let src = img.into_rgba8();
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let cos = angle.cos().abs();
    let sin = angle.sin().abs();
    let bb_w = (sw * cos + sh * sin).ceil() as u32;
    let bb_h = (sh * cos + sw * sin).ceil() as u32;

    let (dw, dh) = (bb_w as f32, bb_h as f32);
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    let mut dst = image::RgbaImage::new(bb_w, bb_h);
    for (x, y, px) in dst.enumerate_pixels_mut() {
        let dx = x as f32 - dw / 2.0;
        let dy = y as f32 - dh / 2.0;
        // Inverse rotation maps this output pixel back into source space.
        let sx = dx * cos_a + dy * sin_a + sw / 2.0;
        let sy = -dx * sin_a + dy * cos_a + sh / 2.0;

        // Anti-aliased edge: up to one pixel outside the source, opacity fades
        // with the distance to it; pixels inside are unchanged.
        let outside = (-sx).max(-sy).max(sx - (sw - 1.0)).max(sy - (sh - 1.0));
        if outside >= 1.0 {
            *px = image::Rgba([0, 0, 0, 0]);
            continue;
        }
        let coverage = (1.0 - outside).min(1.0);
        let sx = sx.clamp(0.0, sw - 1.0);
        let sy = sy.clamp(0.0, sh - 1.0);

        let x0 = sx.floor() as u32;
        let y0 = sy.floor() as u32;
        let x1 = (x0 + 1).min(src.width() - 1);
        let y1 = (y0 + 1).min(src.height() - 1);
        let fx = sx - x0 as f32;
        let fy = sy - y0 as f32;

        let p00 = src.get_pixel(x0, y0).0;
        let p10 = src.get_pixel(x1, y0).0;
        let p01 = src.get_pixel(x0, y1).0;
        let p11 = src.get_pixel(x1, y1).0;

        let mut out = [0u8; 4];
        for i in 0..4 {
            let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
            let bottom = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
            out[i] = (top * (1.0 - fy) + bottom * fy).round() as u8;
        }
        out[3] = (out[3] as f32 * coverage).round() as u8;
        *px = image::Rgba(out);
    }

    DynamicImage::ImageRgba8(dst)
}

/// Decode a source image upright: formats the `image` crate cannot read
/// (RAW, HEIC, JXL, ...) go through their generated preview, which is already
/// oriented; the others are decoded directly and the EXIF orientation applied.
pub(crate) async fn load_oriented_image(
    file_path: &str,
    orientation: i32,
) -> Result<DynamicImage, String> {
    t_utils::require_local_content(Path::new(file_path)).map_err(|e| e.to_string())?;
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);
    if crate::t_affinity::is_affinity_path(file_path) {
        let path = file_path.to_string();
        return tauri::async_runtime::spawn_blocking(move || {
            crate::t_affinity::decode_preview(&path)
                .ok_or_else(|| "Missing embedded Affinity preview".to_string())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    // decoding is CPU-bound: keep it off the async runtime threads
    if should_generate_preview_for_file(file_path, file_type) {
        let preview = get_generated_preview_bytes(file_path)
            .await?
            .ok_or_else(|| "Failed to resolve editable preview image".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            image::load_from_memory(&preview)
                .map_err(|e| format!("Failed to decode editable preview image: {}", e))
        })
        .await
        .map_err(|e| format!("Failed to join decode task: {}", e))?
    } else {
        let path = file_path.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            // Prefer the file signature, as File Info does for its format label.
            let img = ImageReader::open(&path)
                .and_then(|reader| reader.with_guessed_format())
                .map_err(|e| e.to_string())?
                .decode()
                .map_err(|e| e.to_string())?;
            Ok(apply_orientation(img, orientation))
        })
        .await
        .map_err(|e| format!("Failed to join decode task: {}", e))?
    }
}

/// get an edited image
async fn get_edited_image(params: &EditParams) -> Result<DynamicImage, String> {
    if is_preview_only_document(&params.source_file_path) {
        return Err("This document format only supports preview".to_string());
    }
    let mut img = load_oriented_image(&params.source_file_path, params.orientation).await?;

    // 1. Flip
    if params.flip_horizontal {
        img = img.fliph();
    }
    if params.flip_vertical {
        img = img.flipv();
    }

    // 2. Rotate (90° multiples)
    match params.rotate {
        90 => img = img.rotate90(),
        180 => img = img.rotate180(),
        270 => img = img.rotate270(),
        -90 => img = img.rotate270(),
        -180 => img = img.rotate180(),
        -270 => img = img.rotate90(),
        _ => {}
    }

    // 2b. Arbitrary-angle rotation (straighten), producing an axis-aligned
    // bounding box that the subsequent crop indexes into. Offloaded to a
    // blocking thread: it is a CPU-heavy full-image pass that should not occupy
    // the async executor (matches the project's spawn_blocking convention).
    if params.crop_angle != 0.0 {
        let angle = params.crop_angle;
        img = tauri::async_runtime::spawn_blocking(move || rotate_arbitrary(img, angle))
            .await
            .map_err(|e| format!("Failed to join rotate task: {}", e))?;
    }

    // 3. Crop
    if params.crop.width > 0 && params.crop.height > 0 {
        img = img.crop_imm(
            params.crop.x,
            params.crop.y,
            params.crop.width,
            params.crop.height,
        );
    }

    // 4. Resize
    if let (Some(w), Some(h)) = (params.resize.width, params.resize.height) {
        if w > 0 && h > 0 {
            img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
        }
    }

    // 5. Adjustments & Filters
    // NOTE: These implementations match CSS filter spec so preview == saved result.

    // Brightness: CSS brightness(X%) multiplies each channel by X/100.
    // Frontend sends -100..100, meaning factor = (100 + b) / 100.
    if let Some(b) = params.brightness {
        if b != 0 {
            let factor = (100 + b) as f32 / 100.0;
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                pixel[0] = (pixel[0] as f32 * factor).clamp(0.0, 255.0) as u8;
                pixel[1] = (pixel[1] as f32 * factor).clamp(0.0, 255.0) as u8;
                pixel[2] = (pixel[2] as f32 * factor).clamp(0.0, 255.0) as u8;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Contrast: CSS contrast(X%) scales deviation from 128 gray by X/100.
    // Frontend sends -100..100, meaning factor = (100 + c) / 100.
    if let Some(c) = params.contrast {
        if c != 0.0 {
            let factor = (100.0 + c) / 100.0;
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                pixel[0] = ((pixel[0] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
                pixel[1] = ((pixel[1] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
                pixel[2] = ((pixel[2] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Blur
    if let Some(sigma) = params.blur {
        if sigma > 0.0 {
            img = img.blur(sigma);
        }
    }

    // Hue Rotate
    if let Some(hue) = params.hue_rotate {
        if hue != 0 {
            img = img.huerotate(hue);
        }
    }

    // Saturation
    if let Some(saturation) = params.saturation {
        if (saturation - 1.0).abs() > f32::EPSILON {
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                let r = pixel[0] as f32;
                let g = pixel[1] as f32;
                let b = pixel[2] as f32;

                // Simple saturation: blend pixel with its grayscale value
                // Luma = 0.299 * R + 0.587 * G + 0.114 * B
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;

                let new_r = (luma + saturation * (r - luma)).clamp(0.0, 255.0) as u8;
                let new_g = (luma + saturation * (g - luma)).clamp(0.0, 255.0) as u8;
                let new_b = (luma + saturation * (b - luma)).clamp(0.0, 255.0) as u8;

                pixel[0] = new_r;
                pixel[1] = new_g;
                pixel[2] = new_b;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Filter
    if let Some(filter) = &params.filter {
        match filter.as_str() {
            "grayscale" => {
                img = DynamicImage::ImageLuma8(img.to_luma8());
            }
            "invert" => {
                img.invert();
            }
            "sepia" => {
                // Manual Sepia implementation since standard image crate might not have it exposed simply
                // Formula:
                // R = (r * 0.393) + (g * 0.769) + (b * 0.189)
                // G = (r * 0.349) + (g * 0.686) + (b * 0.168)
                // B = (r * 0.272) + (g * 0.534) + (b * 0.131)
                let mut rgba = img.to_rgba8();
                for pixel in rgba.pixels_mut() {
                    let r = pixel[0] as f32;
                    let g = pixel[1] as f32;
                    let b = pixel[2] as f32;

                    let new_r = (r * 0.393 + g * 0.769 + b * 0.189).min(255.0) as u8;
                    let new_g = (r * 0.349 + g * 0.686 + b * 0.168).min(255.0) as u8;
                    let new_b = (r * 0.272 + g * 0.534 + b * 0.131).min(255.0) as u8;

                    pixel[0] = new_r;
                    pixel[1] = new_g;
                    pixel[2] = new_b;
                    // alpha unchanged
                }
                img = DynamicImage::ImageRgba8(rgba);
            }
            _ => {}
        }
    }

    Ok(img)
}

#[cfg(target_os = "macos")]
pub fn get_thumbnail_with_sips(
    file_path: &str,
    thumbnail_size: u32,
) -> Result<Option<Vec<u8>>, String> {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let temp_dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .subsec_nanos();
    let temp_file = temp_dir.join(format!("thumb_{}.jpg", nanos));
    let temp_output = temp_file.to_str().ok_or("Invalid temp path")?;

    let output = Command::new("sips")
        .arg("--resampleHeight")
        .arg(thumbnail_size.to_string())
        .arg("-s")
        .arg("format")
        .arg("jpeg")
        .arg(file_path)
        .arg("--out")
        .arg(temp_output)
        .output()
        .map_err(|e| format!("Failed to run sips: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "sips failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let data = fs::read(&temp_file).map_err(|e| format!("Failed to read temp file: {}", e))?;
    let _ = fs::remove_file(temp_file);

    Ok(Some(data))
}

#[cfg(target_os = "macos")]
pub fn get_dimensions_with_sips(file_path: &str) -> Result<Option<(u32, u32)>, String> {
    let output = Command::new("sips")
        .arg("-g")
        .arg("pixelWidth")
        .arg("-g")
        .arg("pixelHeight")
        .arg(file_path)
        .output()
        .map_err(|e| format!("Failed to run sips for dimensions: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "sips dimension probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("pixelWidth:") {
            width = value.trim().parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("pixelHeight:") {
            height = value.trim().parse::<u32>().ok();
        }
    }

    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Ok(Some((width, height))),
        _ => Ok(None),
    }
}

const FILE_IMAGE_RESULT_CACHE_MAX: usize = 8;

#[derive(Clone)]
struct FileImageCacheEntry {
    signature: (u64, u128),
    prefer_embedded_raw_preview: RawDisplayOptions,
    data: Vec<u8>,
}

struct FileImageResultCache {
    entries: HashMap<String, FileImageCacheEntry>,
    order: VecDeque<String>,
}

impl FileImageResultCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(
        &mut self,
        file_path: &str,
        signature: (u64, u128),
        prefer_embedded_raw_preview: RawDisplayOptions,
    ) -> Option<Vec<u8>> {
        let entry = self.entries.get(file_path)?;
        if entry.signature != signature
            || entry.prefer_embedded_raw_preview != prefer_embedded_raw_preview
        {
            self.entries.remove(file_path);
            self.order.retain(|item| item != file_path);
            return None;
        }

        self.order.retain(|item| item != file_path);
        self.order.push_back(file_path.to_string());
        Some(entry.data.clone())
    }

    fn insert(
        &mut self,
        file_path: String,
        signature: (u64, u128),
        prefer_embedded_raw_preview: RawDisplayOptions,
        data: Vec<u8>,
    ) {
        self.entries.insert(
            file_path.clone(),
            FileImageCacheEntry {
                signature,
                prefer_embedded_raw_preview,
                data,
            },
        );
        self.order.retain(|item| item != &file_path);
        self.order.push_back(file_path);

        while self.order.len() > FILE_IMAGE_RESULT_CACHE_MAX {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

static FILE_IMAGE_RESULT_CACHE: Lazy<Mutex<FileImageResultCache>> =
    Lazy::new(|| Mutex::new(FileImageResultCache::new()));

/// Read an existing preview without touching an unavailable original or decoding.
pub fn get_cached_preview_bytes(path: &str, options: RawDisplayOptions) -> Option<Vec<u8>> {
    if let Ok(cache) = RAW_PREVIEW_CACHE.lock() {
        if let Some(entry) = cache
            .iter()
            .rev()
            .find(|entry| entry.0 == path && entry.2 == options)
            .or_else(|| cache.iter().rev().find(|entry| entry.0 == path))
        {
            return Some(entry.3 .0.clone());
        }
    }
    FILE_IMAGE_RESULT_CACHE
        .lock()
        .ok()?
        .entries
        .get(path)
        .map(|entry| entry.data.clone())
}

fn get_file_signature(file_path: &str) -> Result<(u64, u128), String> {
    let metadata = fs::metadata(file_path)
        .map_err(|e| format!("Failed to read file metadata for cache: {}", e))?;
    let modified = metadata
        .modified()
        .map_err(|e| format!("Failed to read file modified time for cache: {}", e))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Invalid file modified time for cache: {}", e))?
        .as_millis();
    Ok((metadata.len(), modified))
}

pub async fn get_file_image_bytes_cached(
    file_path: &str,
    prefer_embedded_raw_preview: RawDisplayOptions,
) -> Result<Vec<u8>, String> {
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);
    let cache_signature = if should_generate_preview_for_file(file_path, file_type) {
        Some(get_file_signature(file_path)?)
    } else {
        None
    };

    if let Some(signature) = cache_signature {
        if let Ok(mut cache) = FILE_IMAGE_RESULT_CACHE.lock() {
            if let Some(cached) = cache.get(file_path, signature, prefer_embedded_raw_preview) {
                return Ok(cached);
            }
        }
    }

    t_utils::require_local_content(Path::new(file_path)).map_err(|e| e.to_string())?;
    let image_data = if file_type == 3 {
        get_raw_preview_image(file_path, prefer_embedded_raw_preview)?
            .ok_or_else(|| format!("Failed to resolve RAW preview image: {}", file_path))?
    } else if crate::t_affinity::is_affinity_path(file_path) {
        let path = file_path.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            crate::t_affinity::preview_png(&path).unwrap_or_else(|| {
                include_bytes!("../../src-vite/src/assets/images/image-file.png").to_vec()
            })
        })
        .await
        .map_err(|e| e.to_string())?
    } else if t_jxl::is_jxl_path(file_path) {
        t_jxl::get_jxl_preview_image(file_path, 4096)?
            .ok_or_else(|| format!("Failed to resolve JXL preview image: {}", file_path))?
    } else if is_heic_path(file_path) {
        crate::t_heif::get_heif_preview(file_path, get_image_orientation(file_path), 4096)?
            .ok_or_else(|| format!("Failed to resolve HEIC preview image: {}", file_path))?
    } else if is_ffmpeg_backed_image_path(file_path) {
        let native_psd = if crate::t_psd::is_psd_path(file_path) {
            psd_viewer_preview(file_path).await
        } else {
            None
        };
        match native_psd {
            Some(data) => data,
            None => crate::t_video::get_video_thumbnail(file_path, 4096, None, None)
                .await?
                .ok_or_else(|| {
                    format!(
                        "Failed to resolve FFmpeg-backed preview image: {}",
                        file_path
                    )
                })?,
        }
    } else if cfg!(target_os = "linux") && is_avif_path(file_path) {
        get_image_thumbnail(file_path, get_image_orientation(file_path), 4096)?
            .ok_or_else(|| format!("Failed to resolve AVIF preview image: {}", file_path))?
    } else if crate::t_libraw::is_tiff_path(file_path) {
        if let Some(data) = tiff_viewer_preview(file_path).await {
            data
        } else {
        match get_raw_preview_image(file_path, RawDisplayOptions::rendered_bright()) {
            Ok(Some(data)) => data,
            _ => tokio::fs::read(file_path)
                .await
                .map_err(|e| format!("Failed to read the image: {}", e))?,
        }
        }
    } else if let Some(data) = png_viewer_preview(file_path).await {
        data
    } else {
        tokio::fs::read(file_path)
            .await
            .map_err(|e| format!("Failed to read the image: {}", e))?
    };

    if let Some(signature) = cache_signature {
        if let Ok(mut cache) = FILE_IMAGE_RESULT_CACHE.lock() {
            cache.insert(
                file_path.to_string(),
                signature,
                prefer_embedded_raw_preview,
                image_data.clone(),
            );
        }
    }

    Ok(image_data)
}

fn get_embedded_preview(path: &str) -> Option<Vec<u8>> {
    if let Ok(Some(data)) = t_libraw::get_embedded_raw_preview_image(path) {
        return Some(data);
    }
    // Some cameras expose previews that LibRaw cannot extract. Preserve the
    // existing JPEG extraction fallback, with accurate source metadata.
    if let Ok(Some(preview)) = select_embedded_jpeg_for_preview(path) {
        let (raw_width, raw_height, _) = t_libraw::get_raw_dimensions_with_flip(path).unwrap_or((1024, 512, 0));
        if t_libraw::embedded_preview_is_large_enough(preview.width, preview.height, raw_width, raw_height)
            && let Ok(image) = image::load_from_memory(&preview.data) {
            let image = apply_orientation(image, preview.orientation);
            let data = crate::t_jpeg::encode_rgb8(&image.to_rgb8(), 85).ok()?;
            return Some(data);
        }
    }
    None
}

fn get_raw_preview_with_source(path: &str, options: RawDisplayOptions, full_size: bool) -> Result<RawPreviewResult, String> {
    if options.embedded() {
        if let Some(data) = get_embedded_preview(path) {
            return Ok((data, "embedded", false));
        }
    }
    let rendered = RawDisplayOptions { mode: crate::t_raw_display::RawPreviewMode::Rendered, ..options };
    match t_libraw::get_raw_preview_with_size(path, rendered, full_size) {
        Ok((data, source, _)) => Ok((data, source, options.embedded())),
        Err(error) => {
            if !options.embedded() {
                if let Some(data) = get_embedded_preview(path) {
                    return Ok((data, "embedded", false));
                }
            }
            // Thumbnail and editing paths also accept files whose contents do
            // not match their RAW extension. Keep browsing's final fallback.
            let image = ImageReader::open(path).ok()
                .and_then(|reader| reader.with_guessed_format().ok())
                .and_then(|reader| reader.decode().ok());
            if let Some(image) = image {
                let data = resize_dynamic_image_to_jpeg(image, get_image_orientation(path), 4096)?;
                return Ok((data, "image", false));
            }
            match get_raw_preview_fallback_image(path)? {
                Some(data) => Ok((data, "image", false)),
                None => Err(error),
            }
        },
    }
}

// Keep decoder work off the async runtime and bound concurrent full RAW previews.
type RawPreviewResult = (Vec<u8>, &'static str, bool);
static RAW_PREVIEW_CACHE: Lazy<Mutex<VecDeque<(String, (u64, u128), RawDisplayOptions, RawPreviewResult, bool)>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static RAW_PREVIEW_PERMITS: Lazy<Vec<std::sync::Arc<tokio::sync::Semaphore>>> = Lazy::new(|| (0..2).map(|_| std::sync::Arc::new(tokio::sync::Semaphore::new(1))).collect());
static RAW_PREVIEW_LOCKS: Lazy<Vec<tokio::sync::Mutex<()>>> = Lazy::new(|| (0..64).map(|_| tokio::sync::Mutex::new(())).collect());
static RAW_PREVIEW_REQUESTS: Lazy<Mutex<HashMap<String, std::sync::Weak<std::sync::atomic::AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn register_raw_preview_request(viewer: &str) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    if let Ok(mut requests) = RAW_PREVIEW_REQUESTS.lock() {
        requests.retain(|_, request| request.strong_count() > 0);
        if let Some(previous) = requests.insert(viewer.to_string(), std::sync::Arc::downgrade(&active)).and_then(|request| request.upgrade()) {
            previous.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    active
}

fn raw_preview_disk_path(path: &str, signature: (u64, u128), options: RawDisplayOptions, full_size: bool) -> Result<std::path::PathBuf, String> {
    let key = blake3::hash(format!("raw-preview-v6:{}:{path}:{signature:?}:{options:?}:{full_size}", options.cache_tag()).as_bytes());
    Ok(crate::t_config::get_app_cache_dir()?.join("raw-previews").join(key.to_hex().as_str()))
}

fn read_raw_preview_disk(path: &std::path::Path) -> Option<RawPreviewResult> {
    let bytes = fs::read(path).ok()?;
    let source = match *bytes.first()? { 1 => "embedded", 2 => "rendered", 3 => "brightened", 4 => "image", _ => return None };
    let unavailable = *bytes.get(1)? != 0;
    let data = bytes.get(2..)?;
    if !data.starts_with(&[0xff, 0xd8]) || !data.ends_with(&[0xff, 0xd9]) { return None; }
    Some((data.to_vec(), source, unavailable))
}

fn write_raw_preview_disk(path: &std::path::Path, result: &RawPreviewResult) {
    let Some(dir) = path.parent() else { return; };
    if fs::create_dir_all(dir).is_err() { return; }
    let mut bytes = vec![match result.1 { "embedded" => 1, "brightened" => 3, "image" => 4, _ => 2 }, result.2 as u8];
    bytes.extend_from_slice(&result.0);
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    if fs::write(&temp, bytes).is_ok() { let _ = fs::rename(&temp, path); }
    let _ = fs::remove_file(&temp);
    let mut entries: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|entry| {
        if entry.path().extension().is_some() { return None; }
        let meta = entry.metadata().ok()?;
        Some((entry.path(), meta.len(), meta.modified().ok()?))
    }).collect();
    let mut size: u64 = entries.iter().map(|entry| entry.1).sum();
    entries.sort_by_key(|entry| entry.2);
    for (file, len, _) in entries {
        if size <= 512 * 1024 * 1024 { break; }
        if fs::remove_file(file).is_ok() { size = size.saturating_sub(len); }
    }
}

pub async fn get_raw_preview_cached(path: &str, options: RawDisplayOptions, full_size: bool, active: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) -> Result<RawPreviewResult, String> {
    let signature = get_file_signature(path)?;
    let disk = raw_preview_disk_path(path, signature, options, full_size).ok();
    let hash = blake3::hash(format!("{path}:{options:?}:{full_size}").as_bytes());
    let _request = RAW_PREVIEW_LOCKS[hash.as_bytes()[0] as usize % 64].lock().await;
    let check_active = || {
        if active.as_ref().is_some_and(|active| !active.load(std::sync::atomic::Ordering::Relaxed)) {
            Err("RAW preview superseded".to_string())
        } else { Ok(()) }
    };
    check_active()?;
    if let Ok(cache) = RAW_PREVIEW_CACHE.lock() {
        if let Some(entry) = cache.iter().find(|entry| entry.0 == path && entry.1 == signature && entry.2 == options && entry.4 == full_size) {
            return Ok(entry.3.clone());
        }
    }
    let cached = if let Some(disk) = disk.clone() {
        tauri::async_runtime::spawn_blocking(move || read_raw_preview_disk(&disk)).await.map_err(|e| e.to_string())?
    } else { None };
    let result = if let Some(cached) = cached { cached } else {
        let permit = RAW_PREVIEW_PERMITS[full_size as usize].clone().acquire_owned().await.map_err(|e| e.to_string())?;
        check_active()?;
        let path = path.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            t_utils::require_local_content(Path::new(&path)).map_err(|e| e.to_string())?;
            let result = get_raw_preview_with_source(&path, options, full_size)?;
            if options.embedded() || result.1 != "embedded" {
                if let Some(disk) = disk { write_raw_preview_disk(&disk, &result); }
            }
            Ok::<_, String>(result)
        }).await.map_err(|e| e.to_string())??
    };
    if !options.embedded() && result.1 == "embedded" { return Ok(result); }
    if let Ok(mut cache) = RAW_PREVIEW_CACHE.lock() {
        cache.retain(|entry| entry.0 != path || (entry.1 == signature && (entry.2 != options || entry.4 != full_size)));
        cache.push_back((path.to_string(), signature, options, result.clone(), full_size));
        while cache.len() > 8 { cache.pop_front(); }
    }
    Ok(result)
}

#[cfg(test)]
mod offline_preview_tests {
    use super::*;

    #[test]
    fn raw_preview_disk_cache_invalidates_legacy_orientation_results() {
        let path = "/fixture/pentax/sample.pef";
        let signature = (123, 456);
        let options = RawDisplayOptions::default();
        let legacy = blake3::hash(format!("raw-preview-v6:{path}:{signature:?}:{options:?}:false").as_bytes());
        let current = raw_preview_disk_path(path, signature, options, false).unwrap();
        assert_ne!(current.file_name().unwrap().to_str().unwrap(), legacy.to_hex().as_str());
        assert_ne!(current, raw_preview_disk_path(path, signature, options, true).unwrap());
        assert_ne!(current, raw_preview_disk_path(path, signature, RawDisplayOptions::rendered_bright(), false).unwrap());
    }

    #[test]
    #[ignore = "requires LAP_RAW_ORIENTATION_MANIFEST listing local RAW samples"]
    fn raw_fixture_batch_orientation() {
        let manifest = std::env::var("LAP_RAW_ORIENTATION_MANIFEST").unwrap();
        let output = std::env::temp_dir().join("lap-raw-orientation");
        fs::create_dir_all(&output).unwrap();
        for path in fs::read_to_string(manifest).unwrap().lines() {
            let name = Path::new(path).file_name().unwrap().to_str().unwrap();
            println!("FILE {name}, type={:?}, EXIF={}", t_utils::get_file_type(path), get_image_orientation(path));
            t_libraw::inspect_raw_fixture_orientation(path);
            for mode in [crate::t_raw_display::RawPreviewMode::Embedded, crate::t_raw_display::RawPreviewMode::Rendered] {
                let options = RawDisplayOptions { mode, prefer_pair: false, auto_bright: false };
                let thumb = if t_utils::get_file_type(path) == Some(3) {
                    get_raw_thumbnail(path, get_image_orientation(path), 512, options)
                } else { get_image_thumbnail(path, get_image_orientation(path), 512) }.unwrap().unwrap();
                let (preview, source) = if t_utils::get_file_type(path) == Some(3) {
                    let (data, source, _) = get_raw_preview_with_source(path, options, false).unwrap();
                    (data, source)
                } else { (tauri::async_runtime::block_on(get_file_image_bytes_cached(path, options)).unwrap(), "tiff") };
                let a = image::load_from_memory(&thumb).unwrap();
                let b = image::load_from_memory(&preview).unwrap();
                println!("  {mode:?}: thumb={}x{} preview={}x{} source={source}",a.width(),a.height(),b.width(),b.height());
                assert_eq!(a.width() > a.height(), b.width() > b.height(), "{name}: {mode:?} orientation differs");
                fs::write(output.join(format!("{name}-{mode:?}-thumb.jpg")), thumb).unwrap();
                fs::write(output.join(format!("{name}-{mode:?}-preview.jpg")), preview).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "requires LAP_RAW_PREVIEW_FIXTURE pointing to a portrait Nikon E5700 NEF"]
    fn nikon_e5700_thumbnail_matches_preview_orientation() {
        let path = std::env::var("LAP_RAW_PREVIEW_FIXTURE").unwrap();
        for mode in [crate::t_raw_display::RawPreviewMode::Embedded, crate::t_raw_display::RawPreviewMode::Rendered] {
            let options = RawDisplayOptions { mode, prefer_pair: false, auto_bright: false };
            let thumbnail = get_raw_thumbnail(&path, get_image_orientation(&path), 512, options).unwrap().unwrap();
            let thumbnail_image = image::load_from_memory(&thumbnail).unwrap();
            let (preview, source, _) = get_raw_preview_with_source(&path, options, false).unwrap();
            let preview_image = image::load_from_memory(&preview).unwrap();
            println!("{mode:?}: thumbnail {}x{}, preview {}x{} ({source})",
                thumbnail_image.width(), thumbnail_image.height(), preview_image.width(), preview_image.height());
            assert!(preview_image.height() > preview_image.width());
            assert!(thumbnail_image.height() > thumbnail_image.width());
        }
    }

    #[test]
    #[ignore = "requires LAP_RAW_PREVIEW_FIXTURE pointing to an unsupported RAW file"]
    fn editable_raw_preview_falls_back_to_embedded_pixels() {
        let path = std::env::var("LAP_RAW_PREVIEW_FIXTURE").unwrap();
        assert!(t_libraw::get_raw_preview_image(&path, RawDisplayOptions::rendered_bright()).is_err());
        let embedded = get_embedded_preview(&path).expect("embedded preview must be available");
        let editing = tauri::async_runtime::block_on(get_generated_preview_bytes(&path))
            .unwrap().unwrap();
        let image = image::load_from_memory(&editing).unwrap();
        println!("Editable embedded preview: {} × {}", image.width(), image.height());
        assert_eq!(editing, embedded);
        // Export/crop must decode the same oriented pixels as the editing preview.
        let export = tauri::async_runtime::block_on(load_oriented_image(&path, 1)).unwrap();
        assert_eq!(export.to_rgb8(), image.to_rgb8());
    }

    #[test]
    fn editable_png_with_jpeg_extension_uses_file_signature() {
        let dir = std::env::temp_dir().join(format!("lap-edit-format-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.jpg");
        let pixels = image::RgbaImage::from_pixel(4, 3, image::Rgba([60, 120, 180, 128]));
        pixels.save_with_format(&path, ImageFormat::Png).unwrap();

        let result = tauri::async_runtime::block_on(load_oriented_image(path.to_str().unwrap(), 1));
        fs::remove_dir_all(dir).unwrap();
        assert_eq!(result.unwrap().into_rgba8(), pixels);
    }

    #[test]
    fn raw_extension_with_jpeg_contents_uses_generic_preview_fallback() {
        let dir = std::env::temp_dir().join(format!("lap-raw-fallback-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let pixels = image::RgbImage::from_pixel(32, 24, image::Rgb([60, 120, 180]));
        let jpeg = crate::t_jpeg::encode_rgb8(&pixels, 85).unwrap();
        for extension in ["nef", "cr2"] {
            let path = dir.join(format!("sample.{extension}"));
            fs::write(&path, &jpeg).unwrap();
            let path = path.to_str().unwrap();
            for mode in [crate::t_raw_display::RawPreviewMode::Embedded, crate::t_raw_display::RawPreviewMode::Rendered] {
                let options = RawDisplayOptions { mode, auto_bright: true, prefer_pair: false };
                for full_size in [false, true] {
                    let (data, source, unavailable) = get_raw_preview_with_source(path, options, full_size).unwrap();
                    let decoded = image::load_from_memory(&data).unwrap();
                    assert_eq!((decoded.width(), decoded.height()), (32, 24));
                    assert_eq!(source, "image");
                    assert!(!unavailable);
                }
            }
            fs::write(path, b"not an image").unwrap();
            assert!(get_raw_preview_with_source(path, RawDisplayOptions::rendered_bright(), false).is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn raw_disk_cache_preserves_source_and_rejects_incomplete_data() {
        let viewer = uuid::Uuid::new_v4().to_string();
        let previous = register_raw_preview_request(&viewer);
        let current = register_raw_preview_request(&viewer);
        assert!(!previous.load(std::sync::atomic::Ordering::Relaxed));
        assert!(current.load(std::sync::atomic::Ordering::Relaxed));
        let dir = std::env::temp_dir().join(format!("lap-raw-cache-{}", uuid::Uuid::new_v4()));
        let path = dir.join("preview");
        let pixels = image::RgbImage::new(2, 2);
        let jpeg = crate::t_jpeg::encode_rgb8(&pixels, 85).unwrap();
        for source in ["embedded", "rendered", "brightened", "image"] {
            let result = (jpeg.clone(), source, source != "embedded");
            write_raw_preview_disk(&path, &result);
            assert_eq!(read_raw_preview_disk(&path), Some(result));
        }
        fs::write(&path, [2, 0, 0xff, 0xd8]).unwrap();
        assert!(read_raw_preview_disk(&path).is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn offline_preview_reads_existing_pixels_without_source_metadata_or_regeneration() {
        let path = format!("/lap-test-unmounted-volume/{}.raw", uuid::Uuid::new_v4());
        assert!(!std::path::Path::new(&path).exists());
        let options = RawDisplayOptions::default();
        assert!(get_cached_preview_bytes(&path, options).is_none());
        FILE_IMAGE_RESULT_CACHE.lock().unwrap().insert(
            path.clone(),
            (3, 123),
            options,
            vec![1, 2, 3],
        );
        assert_eq!(
            get_cached_preview_bytes(&path, options),
            Some(vec![1, 2, 3])
        );
        // Offline browsing may use an existing preview from a different RAW mode.
        assert_eq!(
            get_cached_preview_bytes(&path, RawDisplayOptions::rendered_bright()),
            Some(vec![1, 2, 3])
        );
        RAW_PREVIEW_CACHE.lock().unwrap().push_back((
            path.clone(),
            (3, 123),
            options,
            (vec![4, 5, 6], "embedded", false),
            false,
        ));
        assert_eq!(
            get_cached_preview_bytes(&path, options),
            Some(vec![4, 5, 6])
        );
        assert_eq!(
            get_cached_preview_bytes(&path, RawDisplayOptions::rendered_bright()),
            Some(vec![4, 5, 6])
        );
        let mut cache = FILE_IMAGE_RESULT_CACHE.lock().unwrap();
        cache.entries.remove(&path);
        cache.order.retain(|entry| entry != &path);
        RAW_PREVIEW_CACHE
            .lock()
            .unwrap()
            .retain(|entry| entry.0 != path);
    }
}
