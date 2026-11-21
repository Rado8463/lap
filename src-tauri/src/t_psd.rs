//! Minimal reader for the merged ("composite") image stored in Photoshop PSD / PSB files.
//!
//! Every PSD/PSB contains a flattened copy of the document (when "Maximize
//! compatibility" is on, which is the default). Reading it directly is much
//! faster than going through FFmpeg and is not limited in resolution.
//!
//! Supported: 8/16-bit grayscale, RGB and CMYK with raw or RLE (PackBits)
//! image data. Anything else returns an error so that callers can fall back.
//! Extra channels (alpha / spot) are ignored.

use image::RgbImage;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub fn is_psd_path(file_path: &str) -> bool {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb"))
}

struct Header {
    is_psb: bool,
    channels: usize,
    height: usize,
    width: usize,
    depth: usize,
    mode: u16,
}

fn read_u16<R: Read>(r: &mut R) -> Result<u16, String> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, String> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u32::from_be_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, String> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u64::from_be_bytes(b))
}

fn read_header<R: Read>(r: &mut R) -> Result<Header, String> {
    let mut sig = [0u8; 4];
    r.read_exact(&mut sig).map_err(|e| e.to_string())?;
    if &sig != b"8BPS" {
        return Err("Not a Photoshop file".to_string());
    }
    let version = read_u16(r)?;
    if version != 1 && version != 2 {
        return Err(format!("Unsupported PSD version {}", version));
    }
    let mut reserved = [0u8; 6];
    r.read_exact(&mut reserved).map_err(|e| e.to_string())?;
    let channels = read_u16(r)? as usize;
    let height = read_u32(r)? as usize;
    let width = read_u32(r)? as usize;
    let depth = read_u16(r)? as usize;
    let mode = read_u16(r)?;
    if width == 0 || height == 0 {
        return Err("Invalid PSD dimensions".to_string());
    }
    Ok(Header {
        is_psb: version == 2,
        channels,
        height,
        width,
        depth,
        mode,
    })
}

/// Reads just the canvas size from the header.
pub fn read_dimensions(file_path: &str) -> Result<(u32, u32), String> {
    let file = File::open(file_path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let header = read_header(&mut reader)?;
    Ok((header.width as u32, header.height as u32))
}

fn unpack_packbits(src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    let mut i = 0usize;
    let mut o = 0usize;
    while o < dst.len() {
        if i >= src.len() {
            return Err("Truncated PackBits row".to_string());
        }
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let len = n as usize + 1;
            if i + len > src.len() || o + len > dst.len() {
                return Err("Corrupt PackBits literal run".to_string());
            }
            dst[o..o + len].copy_from_slice(&src[i..i + len]);
            i += len;
            o += len;
        } else if n != -128 {
            let len = (1 - n as isize) as usize;
            if i >= src.len() || o + len > dst.len() {
                return Err("Corrupt PackBits repeat run".to_string());
            }
            let value = src[i];
            i += 1;
            dst[o..o + len].fill(value);
            o += len;
        }
    }
    Ok(())
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 16)
}

/// Decodes the merged image into 8-bit RGB (alpha ignored).
///
/// Rows are independent once the byte counts are known, so RLE decoding and the
/// final interleaving are spread over all CPU cores.
pub fn decode_composite(file_path: &str) -> Result<RgbImage, String> {
    let file = File::open(file_path).map_err(|e| e.to_string())?;
    let mut r = BufReader::with_capacity(1 << 20, file);
    let h = read_header(&mut r)?;

    if h.depth != 8 && h.depth != 16 {
        return Err(format!("Unsupported PSD bit depth {}", h.depth));
    }
    let needed = match h.mode {
        1 => 1,
        3 => 3,
        4 => 4,
        other => return Err(format!("Unsupported PSD color mode {}", other)),
    };
    if h.channels < needed {
        return Err("PSD has fewer channels than its color mode needs".to_string());
    }

    // Color mode data, image resources.
    for _ in 0..2 {
        let len = read_u32(&mut r)? as i64;
        r.seek(SeekFrom::Current(len)).map_err(|e| e.to_string())?;
    }
    // Layer and mask information.
    let layer_len = if h.is_psb {
        read_u64(&mut r)? as i64
    } else {
        read_u32(&mut r)? as i64
    };
    r.seek(SeekFrom::Current(layer_len)).map_err(|e| e.to_string())?;

    let compression = read_u16(&mut r)?;
    let (w, ht) = (h.width, h.height);
    let bps = h.depth / 8;
    let row_bytes = w * bps;
    let pixels = w
        .checked_mul(ht)
        .ok_or_else(|| "PSD too large".to_string())?;
    let workers = worker_count();

    let mut planes: Vec<Vec<u8>> = (0..needed).map(|_| vec![0u8; pixels]).collect();

    match compression {
        0 => {
            // Raw: channels are stored one after another, row by row.
            let mut plane_raw = vec![0u8; row_bytes * ht];
            for plane in planes.iter_mut() {
                r.read_exact(&mut plane_raw).map_err(|e| e.to_string())?;
                if bps == 1 {
                    plane.copy_from_slice(&plane_raw);
                } else {
                    for (d, chunk) in plane.iter_mut().zip(plane_raw.chunks_exact(bps)) {
                        *d = chunk[0];
                    }
                }
            }
        }
        1 => {
            let total_rows = h.channels * ht;
            let mut counts = Vec::with_capacity(total_rows);
            for _ in 0..total_rows {
                counts.push(if h.is_psb {
                    read_u32(&mut r)? as usize
                } else {
                    read_u16(&mut r)? as usize
                });
            }
            // Rows of the channels we need are contiguous at the start of the data.
            let needed_rows = needed * ht;
            let mut offsets = Vec::with_capacity(needed_rows + 1);
            let mut total = 0usize;
            offsets.push(0);
            for &c in &counts[..needed_rows] {
                total = total
                    .checked_add(c)
                    .ok_or_else(|| "PSD row data too large".to_string())?;
                offsets.push(total);
            }
            let mut packed = vec![0u8; total];
            r.read_exact(&mut packed).map_err(|e| e.to_string())?;

            let mut rows: Vec<(usize, &mut [u8])> = Vec::with_capacity(needed_rows);
            for (c, plane) in planes.iter_mut().enumerate() {
                for (y, row) in plane.chunks_mut(w).enumerate() {
                    rows.push((c * ht + y, row));
                }
            }
            let per_worker = rows.len().div_ceil(workers).max(1);
            let packed_ref = &packed;
            let offsets_ref = &offsets;
            std::thread::scope(|scope| -> Result<(), String> {
                let handles: Vec<_> = rows
                    .chunks_mut(per_worker)
                    .map(|chunk| {
                        scope.spawn(move || -> Result<(), String> {
                            let mut tmp = if bps == 1 { Vec::new() } else { vec![0u8; row_bytes] };
                            for (idx, dst) in chunk.iter_mut() {
                                let src = &packed_ref[offsets_ref[*idx]..offsets_ref[*idx + 1]];
                                if bps == 1 {
                                    unpack_packbits(src, dst)?;
                                } else {
                                    unpack_packbits(src, &mut tmp)?;
                                    for (x, d) in dst.iter_mut().enumerate() {
                                        *d = tmp[x * bps];
                                    }
                                }
                            }
                            Ok(())
                        })
                    })
                    .collect();
                for handle in handles {
                    handle
                        .join()
                        .map_err(|_| "PSD decoder thread panicked".to_string())??;
                }
                Ok(())
            })?;
        }
        other => return Err(format!("Unsupported PSD compression {}", other)),
    }

    let mut rgb = vec![0u8; pixels * 3];
    let mode = h.mode;
    let planes_ref = &planes;
    let rows_per_worker = ht.div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (chunk_index, out) in rgb.chunks_mut(rows_per_worker * w * 3).enumerate() {
            scope.spawn(move || {
                let first_pixel = chunk_index * rows_per_worker * w;
                for (i, px) in out.chunks_exact_mut(3).enumerate() {
                    let p = first_pixel + i;
                    match mode {
                        1 => {
                            let v = planes_ref[0][p];
                            px[0] = v;
                            px[1] = v;
                            px[2] = v;
                        }
                        3 => {
                            px[0] = planes_ref[0][p];
                            px[1] = planes_ref[1][p];
                            px[2] = planes_ref[2][p];
                        }
                        _ => {
                            // CMYK: Photoshop stores inverted ink values (255 = no ink).
                            let k = planes_ref[3][p] as u32;
                            px[0] = ((planes_ref[0][p] as u32 * k + 127) / 255) as u8;
                            px[1] = ((planes_ref[1][p] as u32 * k + 127) / 255) as u8;
                            px[2] = ((planes_ref[2][p] as u32 * k + 127) / 255) as u8;
                        }
                    }
                }
            });
        }
    });

    RgbImage::from_raw(w as u32, ht as u32, rgb).ok_or_else(|| "Failed to build RGB image".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_literal_and_repeat() {
        let src = [0x02, 1, 2, 3, 0xFE, 9];
        let mut dst = [0u8; 6];
        unpack_packbits(&src, &mut dst).unwrap();
        assert_eq!(dst, [1, 2, 3, 9, 9, 9]);
    }

    #[test]
    fn packbits_rejects_truncated_input() {
        let src = [0x05, 1, 2];
        let mut dst = [0u8; 6];
        assert!(unpack_packbits(&src, &mut dst).is_err());
    }
}
