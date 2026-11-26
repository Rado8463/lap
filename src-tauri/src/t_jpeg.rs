use image::RgbImage;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_ulong};

#[link(name = "lap_libraw_shim", kind = "static")]
unsafe extern "C" {
    fn lap_jpeg_encode_rgb8(
        rgb_data: *const u8,
        width: u32,
        height: u32,
        quality: c_int,
        out_data: *mut *mut u8,
        out_len: *mut c_ulong,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_encode_rgb8_ex(
        rgb_data: *const u8,
        width: u32,
        height: u32,
        quality: c_int,
        full_chroma: c_int,
        out_data: *mut *mut u8,
        out_len: *mut c_ulong,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_encode_rgb8_opts(
        rgb_data: *const u8,
        width: u32,
        height: u32,
        quality: c_int,
        sampling: c_int,
        progressive: c_int,
        optimize_coding: c_int,
        out_data: *mut *mut u8,
        out_len: *mut c_ulong,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_decode_rgb8(
        file_path: *const c_char,
        target_width: u32,
        target_height: u32,
        out_width: *mut u32,
        out_height: *mut u32,
        out_data: *mut *mut u8,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_decode_region_rgb8(
        file_path: *const c_char,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        scale_denom: u32,
        out_width: *mut u32,
        out_height: *mut u32,
        out_data: *mut *mut u8,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_decode_parallel_rgb8(
        file_path: *const c_char,
        width: u32,
        height: u32,
        nthreads: u32,
        out: *mut u8,
        out_cap: usize,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_decode_parallel_scaled_rgb8(
        file_path: *const c_char,
        target_width: u32,
        target_height: u32,
        nthreads: u32,
        out_width: *mut u32,
        out_height: *mut u32,
        out_data: *mut *mut u8,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;
    fn lap_jpeg_free_buffer(data: *mut u8);
}

/// Decodes the rectangle (x, y, w, h) (source pixels) of a JPEG at 1/`scale_denom` size
/// (1, 2, 4 or 8). Returns RGB pixels and the size of the decoded rectangle.
pub fn decode_region_rgb8(
    file_path: &str,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    scale_denom: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let mut out_data: *mut u8 = std::ptr::null_mut();
    let mut out_width: u32 = 0;
    let mut out_height: u32 = 0;
    let mut err_buf = vec![0 as c_char; 512];
    let c_path = std::ffi::CString::new(file_path).map_err(|e| e.to_string())?;

    let ok = unsafe {
        lap_jpeg_decode_region_rgb8(
            c_path.as_ptr(),
            x,
            y,
            w,
            h,
            scale_denom,
            &mut out_width,
            &mut out_height,
            &mut out_data,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };

    if ok == 0 {
        let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        return Err(if err.is_empty() {
            "libjpeg-turbo region decode failed".to_string()
        } else {
            format!("libjpeg-turbo region decode failed: {}", err)
        });
    }
    if out_data.is_null() || out_width == 0 || out_height == 0 {
        return Err("libjpeg-turbo returned an empty region".to_string());
    }
    let bytes = unsafe {
        let len = (out_width as usize) * (out_height as usize) * 3;
        let result = std::slice::from_raw_parts(out_data, len).to_vec();
        lap_jpeg_free_buffer(out_data);
        result
    };
    Ok((bytes, out_width, out_height))
}

/// Decodes a whole JPEG (`width` x `height` as stored) on several threads by splitting it at its
/// restart markers. Ok(None) means the file cannot be split (no restart markers, progressive ...);
/// the caller then decodes it the usual way.
pub fn decode_full_parallel(file_path: &str, width: u32, height: u32, threads: u32) -> Result<Option<Vec<u8>>, String> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(3))
        .ok_or_else(|| "JPEG too large".to_string())?;
    let c_path = std::ffi::CString::new(file_path).map_err(|e| e.to_string())?;
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut err_buf = vec![0 as c_char; 512];
    let rc = unsafe {
        lap_jpeg_decode_parallel_rgb8(
            c_path.as_ptr(),
            width,
            height,
            threads,
            out.as_mut_ptr(),
            len,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };
    match rc {
        1 => {
            // The decoder has written every byte of the buffer.
            unsafe { out.set_len(len) };
            Ok(Some(out))
        }
        2 => Ok(None),
        _ => {
            let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }.to_string_lossy().trim().to_string();
            Err(if err.is_empty() { "parallel JPEG decode failed".to_string() } else { err })
        }
    }
}

/// Number of worker threads for the parallel decoders.
pub fn decode_threads() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4).min(32)
}

/// Like `decode_rgb8_scaled` (same DCT downscale), but on several threads for a JPEG with restart
/// markers. Ok(None) means the file cannot be split; the caller then decodes it the usual way.
pub fn decode_scaled_parallel(
    file_path: &str,
    target_width: u32,
    target_height: u32,
    threads: u32,
) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    let c_path = std::ffi::CString::new(file_path).map_err(|e| e.to_string())?;
    let mut out_data: *mut u8 = std::ptr::null_mut();
    let (mut out_width, mut out_height) = (0u32, 0u32);
    let mut err_buf = vec![0 as c_char; 512];
    let rc = unsafe {
        lap_jpeg_decode_parallel_scaled_rgb8(
            c_path.as_ptr(),
            target_width,
            target_height,
            threads,
            &mut out_width,
            &mut out_height,
            &mut out_data,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };
    match rc {
        1 => {
            if out_data.is_null() || out_width == 0 || out_height == 0 {
                return Err("parallel JPEG decode returned empty pixels".to_string());
            }
            let bytes = unsafe {
                let len = (out_width as usize) * (out_height as usize) * 3;
                let v = std::slice::from_raw_parts(out_data, len).to_vec();
                lap_jpeg_free_buffer(out_data);
                v
            };
            Ok(Some((bytes, out_width, out_height)))
        }
        2 => Ok(None),
        _ => {
            let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }.to_string_lossy().trim().to_string();
            Err(if err.is_empty() { "parallel JPEG decode failed".to_string() } else { err })
        }
    }
}

pub fn decode_rgb8_scaled(
    file_path: &str,
    target_width: u32,
    target_height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let mut out_data: *mut u8 = std::ptr::null_mut();
    let mut out_width: u32 = 0;
    let mut out_height: u32 = 0;
    let mut err_buf = vec![0 as c_char; 512];

    let c_path = std::ffi::CString::new(file_path).map_err(|e| e.to_string())?;

    let ok = unsafe {
        lap_jpeg_decode_rgb8(
            c_path.as_ptr(),
            target_width,
            target_height,
            &mut out_width,
            &mut out_height,
            &mut out_data,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };

    if ok == 0 {
        let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        return Err(if err.is_empty() {
            "libjpeg-turbo decode failed".to_string()
        } else {
            format!("libjpeg-turbo decode failed: {}", err)
        });
    }

    if out_data.is_null() || out_width == 0 || out_height == 0 {
        return Err("libjpeg-turbo returned empty pixels".to_string());
    }

    let bytes = unsafe {
        let len = (out_width as usize) * (out_height as usize) * 3;
        let slice = std::slice::from_raw_parts(out_data, len);
        let result = slice.to_vec();
        lap_jpeg_free_buffer(out_data);
        result
    };

    Ok((bytes, out_width, out_height))
}

pub fn encode_rgb8(rgb: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    encode_rgb8_ex(rgb, quality, false)
}

/// Like `encode_rgb8`; `full_chroma` selects 4:4:4 (no chroma subsampling).
pub fn encode_rgb8_ex(rgb: &RgbImage, quality: u8, full_chroma: bool) -> Result<Vec<u8>, String> {
    let mut out_data: *mut u8 = std::ptr::null_mut();
    let mut out_len: c_ulong = 0;
    let mut err_buf = vec![0 as c_char; 512];

    let ok = unsafe {
        lap_jpeg_encode_rgb8_ex(
            rgb.as_raw().as_ptr(),
            rgb.width(),
            rgb.height(),
            quality as c_int,
            full_chroma as c_int,
            &mut out_data,
            &mut out_len,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };

    if ok == 0 {
        let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        return Err(if err.is_empty() {
            "libjpeg-turbo encode failed".to_string()
        } else {
            format!("libjpeg-turbo encode failed: {}", err)
        });
    }

    if out_data.is_null() || out_len == 0 {
        return Err("libjpeg-turbo returned empty JPEG output".to_string());
    }

    let bytes = unsafe {
        let slice = std::slice::from_raw_parts(out_data, out_len as usize);
        let result = slice.to_vec();
        lap_jpeg_free_buffer(out_data);
        result
    };

    Ok(bytes)
}

/// JPEG encode with explicit settings. `sampling`: 0 = 4:4:4, 1 = 4:2:2, 2 = 4:2:0.
pub fn encode_rgb8_opts(
    rgb: &RgbImage,
    quality: u8,
    sampling: u8,
    progressive: bool,
    optimize_coding: bool,
) -> Result<Vec<u8>, String> {
    let mut out_data: *mut u8 = std::ptr::null_mut();
    let mut out_len: c_ulong = 0;
    let mut err_buf = vec![0 as c_char; 512];

    let ok = unsafe {
        lap_jpeg_encode_rgb8_opts(
            rgb.as_raw().as_ptr(),
            rgb.width(),
            rgb.height(),
            quality as c_int,
            sampling as c_int,
            progressive as c_int,
            optimize_coding as c_int,
            &mut out_data,
            &mut out_len,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };

    if ok == 0 {
        let err = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        return Err(if err.is_empty() {
            "libjpeg-turbo encode failed".to_string()
        } else {
            format!("libjpeg-turbo encode failed: {}", err)
        });
    }

    if out_data.is_null() || out_len == 0 {
        return Err("libjpeg-turbo returned empty JPEG output".to_string());
    }

    let bytes = unsafe {
        let slice = std::slice::from_raw_parts(out_data, out_len as usize);
        let result = slice.to_vec();
        lap_jpeg_free_buffer(out_data);
        result
    };

    Ok(bytes)
}
