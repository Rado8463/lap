//! Standalone viewer: Lap started with an image path (e.g. "Open with" in Windows Explorer).
//!
//! This process never touches the user's real library. It runs on a throw-away app-data folder
//! (empty DB, default config, own caches and WebView profile) and registers the opened file and
//! its folder neighbours there, so the regular Image Viewer works on them unchanged.

use crate::t_config;
use crate::t_sqlite::{AFile, AFolder, Album};
use crate::t_utils;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
struct Launch {
    file: String,
    dir: String,
    data_dir: PathBuf,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();
static FOLDER_ID: Mutex<Option<i64>> = Mutex::new(None);

const TEMP_PREFIX: &str = "lap-standalone-";

/// Call first thing in main(): detects the launch argument and redirects all app data.
pub fn init_from_args() {
    let Some(arg) = std::env::args_os().skip(1).find_map(|a| {
        let path = PathBuf::from(a);
        path.is_file().then_some(path)
    }) else {
        return;
    };
    let Ok(file) = arg.canonicalize() else { return };
    let file = strip_verbatim(&file);
    let file_str = file.to_string_lossy().to_string();
    if t_utils::get_file_type(&file_str).is_none() {
        return;
    }
    let Some(dir) = file.parent().map(|p| p.to_string_lossy().to_string()) else {
        return;
    };

    cleanup_stale_dirs();
    let data_dir = std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", std::process::id()));
    if std::fs::create_dir_all(&data_dir).is_err() {
        return;
    }
    t_config::set_data_dir_override(data_dir.clone());
    // Own WebView2 profile: the viewer must not share (and overwrite) the settings in the
    // browser storage of a running Lap.
    #[cfg(target_os = "windows")]
    // SAFETY: runs at the very start of main(); nothing else reads or writes the environment yet.
    unsafe {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", data_dir.join("webview"));
    }
    let _ = LAUNCH.set(Launch { file: file_str, dir, data_dir });
}

pub fn is_active() -> bool {
    LAUNCH.get().is_some()
}

/// Remove the throw-away folder on exit (best effort).
pub fn cleanup() {
    if let Some(launch) = LAUNCH.get() {
        let _ = std::fs::remove_dir_all(&launch.data_dir);
    }
}

/// Folders left behind by a crashed viewer (older than a day) are removed on the next start.
fn cleanup_stale_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    let limit = std::time::Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > limit)
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `\\?\C:\x` -> `C:\x` (canonicalize returns verbatim paths on Windows).
fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[derive(Serialize)]
pub struct StandaloneFiles {
    pub files: Vec<String>,
    pub index: usize,
}

/// Natural, case-insensitive ordering ("img2" before "img10").
fn natural_key(name: &str) -> Vec<(bool, String, u64)> {
    let mut out = Vec::new();
    let mut chars = name.to_lowercase().chars().peekable();
    while let Some(&c) = chars.peek() {
        let digit = c.is_ascii_digit();
        let mut run = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() != digit {
                break;
            }
            run.push(c);
            chars.next();
        }
        if digit {
            out.push((true, String::new(), run.parse::<u64>().unwrap_or(u64::MAX)));
        } else {
            out.push((false, run, 0));
        }
    }
    out
}

/// Present only when this process was started with a file to show.
#[tauri::command]
pub fn get_standalone_launch() -> Option<String> {
    LAUNCH.get().map(|l| l.file.clone())
}

/// Supported files of the opened file's folder (no subfolders) and the opened file's position.
#[tauri::command]
pub fn standalone_list() -> Result<StandaloneFiles, String> {
    let launch = LAUNCH.get().ok_or("Not a standalone viewer")?;
    let mut files: Vec<String> = std::fs::read_dir(&launch.dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path().to_string_lossy().to_string())
        .filter(|p| t_utils::get_file_type(p).is_some())
        .collect();
    files.sort_by_cached_key(|p| {
        natural_key(
            &Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    });
    let opened = launch.file.to_lowercase();
    let index = files
        .iter()
        .position(|p| p.to_lowercase() == opened)
        .unwrap_or_else(|| {
            files.insert(0, launch.file.clone());
            0
        });
    Ok(StandaloneFiles { files, index })
}

/// Register one file in the throw-away DB (on demand, so a big folder is not indexed at once)
/// and return its id for the regular viewer commands.
#[tauri::command]
pub fn standalone_register(app_handle: tauri::AppHandle, file_path: String) -> Result<i64, String> {
    let launch = LAUNCH.get().ok_or("Not a standalone viewer")?;
    let file_type = t_utils::get_file_type(&file_path)
        .ok_or_else(|| format!("Unsupported file type: {file_path}"))?;

    let folder_id = {
        let mut guard = FOLDER_ID.lock().map_err(|_| "Standalone lock poisoned".to_string())?;
        match *guard {
            Some(id) => id,
            None => {
                t_utils::authorize_directory_scope(&app_handle, &launch.dir)?;
                let album = Album::add_album_to_db(&launch.dir)?;
                let album_id = album.id.ok_or("New album has no id")?;
                let folder = AFolder::add_to_db(album_id, &launch.dir)?;
                let id = folder.id.ok_or("New folder has no id")?;
                *guard = Some(id);
                id
            }
        }
    };

    let now = chrono::Utc::now().timestamp_millis();
    let (file, _) = AFile::add_to_db(folder_id, &file_path, file_type, now)?;
    file.id.ok_or_else(|| "Registered file has no id".to_string())
}
