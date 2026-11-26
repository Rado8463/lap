//! JPEG converter: previews the result of a conversion (size and picture) for the current
//! settings and writes the converted file next to the original.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::RgbImage;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Longest side of the two comparison pictures sent to the dialog.
const PREVIEW_MAX_EDGE: u32 = 1600;
/// Quality of the pictures shown in the dialog (they are not the converted file itself).
const PREVIEW_JPEG_QUALITY: u8 = 92;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOptions {
    /// 1..=100
    pub quality: u8,
    /// 0 = 4:4:4 (1:1), 1 = 4:2:2, 2 = 4:2:0
    pub sampling: u8,
    pub progressive: bool,
    pub optimize: bool,
    /// a restart marker after every MCU row
    #[serde(default)]
    pub restart_markers: bool,
    pub keep_metadata: bool,
}

impl ConvertOptions {
    fn key(&self) -> ConvertKey {
        (
            self.quality.clamp(1, 100),
            self.sampling.min(2),
            self.progressive,
            self.optimize,
            self.restart_markers,
        )
    }
}

/// quality, sampling, progressive, optimized, restart markers
type ConvertKey = (u8, u8, bool, bool, bool);

struct Compressed {
    key: ConvertKey,
    bytes: Arc<Vec<u8>>,
    decoded: Arc<RgbImage>,
}

struct Session {
    path: String,
    original: Arc<RgbImage>,
    compressed: Option<Compressed>,
}

static SESSION: Lazy<Mutex<Option<Session>>> = Lazy::new(|| Mutex::new(None));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOpenResult {
    pub width: u32,
    pub height: u32,
    pub original_bytes: u64,
    /// data URL of the fitted original picture
    pub original_preview: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPreviewResult {
    pub bytes: u64,
    /// data URL of the fitted picture decoded back from the converted JPEG
    pub preview: String,
}

fn data_url(jpeg: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg))
}

fn encode_preview(rgb: &RgbImage) -> Result<String, String> {
    let fitted = crate::t_image::resize_rgb_fit(rgb, PREVIEW_MAX_EDGE)?;
    let bytes = crate::t_jpeg::encode_rgb8_ex(&fitted, PREVIEW_JPEG_QUALITY, true)?;
    Ok(data_url(&bytes))
}

fn encode_with(rgb: &RgbImage, options: &ConvertOptions) -> Result<Vec<u8>, String> {
    let (quality, sampling, progressive, optimize, restart) = options.key();
    crate::t_jpeg::encode_rgb8_opts(rgb, quality, sampling, progressive, optimize, restart)
}

/// Loads the picture for the converter and returns its size and a fitted preview.
/// `background` (0xRRGGBB) replaces transparency.
pub async fn convert_open(file_path: String, background: u32) -> Result<ConvertOpenResult, String> {
    let rgb = crate::t_image::decode_for_conversion(&file_path, background & 0xff_ffff).await?;
    let original = Arc::new(rgb);
    let original_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    let for_preview = original.clone();
    let preview = tauri::async_runtime::spawn_blocking(move || encode_preview(&for_preview))
        .await
        .map_err(|e| format!("Failed to join preview task: {}", e))??;
    let (width, height) = original.dimensions();
    if let Ok(mut guard) = SESSION.lock() {
        *guard = Some(Session {
            path: file_path,
            original,
            compressed: None,
        });
    }
    Ok(ConvertOpenResult {
        width,
        height,
        original_bytes,
        original_preview: preview,
    })
}

fn current_original(file_path: &str) -> Result<Arc<RgbImage>, String> {
    let guard = SESSION.lock().map_err(|_| "converter state poisoned".to_string())?;
    match guard.as_ref() {
        Some(session) if session.path == file_path => Ok(session.original.clone()),
        _ => Err("Converter is not open for this file".to_string()),
    }
}

/// Encodes the picture with `options` (exact size) and returns it decoded back, fitted to the dialog.
pub async fn convert_preview(
    file_path: String,
    options: ConvertOptions,
) -> Result<ConvertPreviewResult, String> {
    let original = current_original(&file_path)?;
    let key = options.key();
    let path = file_path.clone();
    let (bytes_len, preview, compressed) = tauri::async_runtime::spawn_blocking(move || {
        let bytes = encode_with(&original, &options)?;
        let mut reader = image::ImageReader::with_format(
            std::io::Cursor::new(&bytes[..]),
            image::ImageFormat::Jpeg,
        );
        reader.no_limits();
        let decoded = reader
            .decode()
            .map_err(|e| format!("Failed to decode converted JPEG: {}", e))?
            .into_rgb8();
        let preview = encode_preview(&decoded)?;
        let len = bytes.len() as u64;
        Ok::<_, String>((
            len,
            preview,
            Compressed {
                key,
                bytes: Arc::new(bytes),
                decoded: Arc::new(decoded),
            },
        ))
    })
    .await
    .map_err(|e| format!("Failed to join convert task: {}", e))??;
    if let Ok(mut guard) = SESSION.lock() {
        if let Some(session) = guard.as_mut() {
            if session.path == path {
                session.compressed = Some(compressed);
            }
        }
    }
    Ok(ConvertPreviewResult {
        bytes: bytes_len,
        preview,
    })
}

/// A 1:1 crop of the original (`converted` = false) or of the last converted result, centred on
/// (cx, cy) in picture pixels and `w` x `h` large (clamped to the picture). Returns a data URL
/// together with the real top-left corner of the crop.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertRegion {
    pub image: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub async fn convert_region(
    file_path: String,
    converted: bool,
    cx: i64,
    cy: i64,
    w: u32,
    h: u32,
) -> Result<ConvertRegion, String> {
    let source: Arc<RgbImage> = {
        let guard = SESSION.lock().map_err(|_| "converter state poisoned".to_string())?;
        let session = match guard.as_ref() {
            Some(session) if session.path == file_path => session,
            _ => return Err("Converter is not open for this file".to_string()),
        };
        if converted {
            session
                .compressed
                .as_ref()
                .map(|c| c.decoded.clone())
                .ok_or_else(|| "No converted picture yet".to_string())?
        } else {
            session.original.clone()
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        let (pw, ph) = source.dimensions();
        let rw = w.clamp(1, pw);
        let rh = h.clamp(1, ph);
        let x = (cx - rw as i64 / 2).clamp(0, (pw - rw) as i64) as u32;
        let y = (cy - rh as i64 / 2).clamp(0, (ph - rh) as i64) as u32;
        let crop = image::imageops::crop_imm(&*source, x, y, rw, rh).to_image();
        let bytes = crate::t_jpeg::encode_rgb8_ex(&crop, 95, true)?;
        Ok(ConvertRegion {
            image: data_url(&bytes),
            x,
            y,
            width: rw,
            height: rh,
        })
    })
    .await
    .map_err(|e| format!("Failed to join region task: {}", e))?
}

/// First free `name.jpg`, `name (1).jpg`, ... next to the original.
fn free_destination(source: &Path) -> PathBuf {
    let dir = source.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let mut candidate = dir.join(format!("{}.jpg", stem));
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{} ({}).jpg", stem, n));
        n += 1;
    }
    candidate
}

/// Writes the converted file next to the original (never overwrites) and returns its path.
pub async fn convert_save(file_path: String, options: ConvertOptions) -> Result<String, String> {
    let key = options.key();
    let cached: Option<Arc<Vec<u8>>> = {
        let guard = SESSION.lock().map_err(|_| "converter state poisoned".to_string())?;
        match guard.as_ref() {
            Some(session) if session.path == file_path => session
                .compressed
                .as_ref()
                .filter(|c| c.key == key)
                .map(|c| c.bytes.clone()),
            _ => return Err("Converter is not open for this file".to_string()),
        }
    };
    let original = current_original(&file_path)?;
    let path = file_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = match cached {
            Some(bytes) => bytes,
            None => Arc::new(encode_with(&original, &options)?),
        };
        write_converted(Path::new(&path), bytes.as_slice(), options.keep_metadata)
    })
    .await
    .map_err(|e| format!("Failed to join save task: {}", e))?
}

/// Writes the JPEG next to the original and copies the metadata (best effort).
fn write_converted(source: &Path, bytes: &[u8], keep_metadata: bool) -> Result<String, String> {
    let dest = free_destination(source);
    std::fs::write(&dest, bytes)
        .map_err(|e| format!("Failed to write {}: {}", dest.display(), e))?;
    if keep_metadata {
        // The converted picture is already saved even when the metadata cannot be copied.
        if let Err(error) = crate::t_image::copy_metadata_to_output(source, &dest) {
            eprintln!("convert: metadata not copied: {}", error);
        }
    }
    Ok(dest.to_string_lossy().to_string())
}

/// Converts one file without the dialog's picture session (batch conversion); returns the new path.
pub async fn convert_file(
    file_path: String,
    options: ConvertOptions,
    background: u32,
) -> Result<String, String> {
    let rgb = crate::t_image::decode_for_conversion(&file_path, background & 0xff_ffff).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = encode_with(&rgb, &options)?;
        drop(rgb);
        write_converted(Path::new(&file_path), &bytes, options.keep_metadata)
    })
    .await
    .map_err(|e| format!("Failed to join convert task: {}", e))?
}

/// Frees the pictures held for the converter dialog.
pub fn convert_close() {
    if let Ok(mut guard) = SESSION.lock() {
        *guard = None;
    }
}
