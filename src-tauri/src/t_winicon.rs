//! Window icon for the classic Windows Alt+Tab switcher.
//!
//! That switcher reads the icon of the window class (and WM_GETICON); a frameless transparent
//! Tauri window may have neither, which shows a generic icon. The icon is set on every window
//! once, both as the window icon and as the class icon.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, SetClassLongPtrW, GCLP_HICON, GCLP_HICONSM};

static HICON_RAW: OnceLock<isize> = OnceLock::new();
static DONE: OnceLock<Mutex<HashSet<isize>>> = OnceLock::new();

fn load_image() -> Option<tauri::image::Image<'static>> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/window-icon.png")).ok()
}

fn make_hicon() -> isize {
    let Some(image) = load_image() else { return 0 };
    let (w, h) = (image.width() as usize, image.height() as usize);
    let rgba = image.rgba();
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return 0;
    }
    let row = ((w + 15) / 16) * 2;
    let mut and_mask = vec![0u8; row * h];
    let mut bgra = Vec::with_capacity(w * h * 4);
    for (i, px) in rgba.chunks_exact(4).take(w * h).enumerate() {
        bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        if px[3] == 0 {
            let (x, y) = (i % w, i / w);
            and_mask[y * row + x / 8] |= 0x80 >> (x % 8);
        }
    }
    unsafe {
        CreateIcon(None, w as i32, h as i32, 1, 32, and_mask.as_ptr(), bgra.as_ptr())
            .map(|icon| icon.0 as isize)
            .unwrap_or(0)
    }
}

pub fn apply(window: &tauri::Window) {
    let Ok(hwnd) = window.hwnd() else { return };
    let raw = hwnd.0 as isize;
    let done = DONE.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let Ok(mut set) = done.lock() else { return };
        if !set.insert(raw) {
            return;
        }
    }
    if let Some(image) = load_image() {
        let _ = window.set_icon(image);
    }
    let icon = *HICON_RAW.get_or_init(make_hicon);
    if icon == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        SetClassLongPtrW(hwnd, GCLP_HICON, icon);
        SetClassLongPtrW(hwnd, GCLP_HICONSM, icon);
    }
}
