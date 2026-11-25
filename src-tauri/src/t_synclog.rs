//! Optional diagnostic log of the startup folder synchronisation
//! (Settings > General > "Log startup folder sync").
//! File: <app data dir>/logs/startup-sync.log (trimmed when it grows over 1 MB).

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Per-file branches taken inside `AFile::add_to_db`, summed per folder by the caller.
pub static PENDING_READS: AtomicU64 = AtomicU64::new(0);
pub static CHANGED_FILES: AtomicU64 = AtomicU64::new(0);
pub static UNCHANGED_RESCANS: AtomicU64 = AtomicU64::new(0);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn count(counter: &AtomicU64) {
    if enabled() {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn take(counter: &AtomicU64) -> u64 {
    counter.swap(0, Ordering::Relaxed)
}

fn log_path() -> Option<PathBuf> {
    crate::t_config::get_app_data_dir().ok().map(|dir| dir.join("logs").join("startup-sync.log"))
}

pub fn log(message: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).map(|m| m.len() > 1_048_576).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), message);
    }
}

#[tauri::command]
pub fn set_sync_logging(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_sync_log_path() -> Option<String> {
    log_path().map(|p| p.to_string_lossy().to_string())
}
