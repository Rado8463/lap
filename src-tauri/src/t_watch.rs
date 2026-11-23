// Watches one folder (the one the user chose to follow) and adds new or changed files to
// the catalog, so photos saved there by other programs show up without a manual scan.

use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::t_sqlite::{AFile, AFolder};
use crate::t_utils;

static WATCHER: Mutex<Option<notify::RecommendedWatcher>> = Mutex::new(None);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchChanged {
    library_id: String,
    album_id: i64,
    folder_path: String,
    added: usize,
    changed: usize,
    removed: usize,
}

/// wait until a file stops growing and can be opened (the writing program may still be busy)
fn wait_until_stable(path: &Path) -> bool {
    let mut last_len: Option<u64> = None;
    for _ in 0..40 {
        let Ok(meta) = std::fs::metadata(path) else { return false };
        let len = meta.len();
        if len > 0 && last_len == Some(len) && std::fs::File::open(path).is_ok() {
            return true;
        }
        last_len = Some(len);
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn process(app: &AppHandle, library_id: &str, album_id: i64, folder: &str, paths: HashSet<PathBuf>) {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut changed_ids: Vec<i64> = Vec::new();
    let mut added_any = false;
    for path in paths {
        let Some(name) = path.file_name() else { continue };
        // keep the catalog's own spelling of the path: folder + file name
        let file_path = Path::new(folder).join(name).to_string_lossy().to_string();
        let Some(file_type) = t_utils::get_file_type(&file_path) else { continue };
        if !Path::new(&file_path).exists() {
            // deleted (or renamed away): drop it from the catalog if it is there
            let result = crate::t_cmds::with_current_library(library_id, || {
                let folder_row = AFolder::add_to_db(album_id, folder)?;
                let folder_id = folder_row.id.ok_or_else(|| "folder without id".to_string())?;
                match AFile::fetch(folder_id, &file_path)?.and_then(|f| f.id) {
                    Some(id) => {
                        AFile::delete(id)?;
                        Ok(true)
                    }
                    None => Ok(false),
                }
            });
            match result {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(e) => eprintln!("watch: failed to remove {}: {}", file_path, e),
            }
            continue;
        }
        if !wait_until_stable(Path::new(&file_path)) {
            continue;
        }
        let result = crate::t_cmds::with_current_library(library_id, || {
            let folder_row = AFolder::add_to_db(album_id, folder)?;
            let folder_id = folder_row.id.ok_or_else(|| "folder without id".to_string())?;
            let existed = AFile::fetch(folder_id, &file_path)?.and_then(|f| f.id);
            let now = chrono::Utc::now().timestamp_millis();
            let (file, _) = AFile::add_to_db(folder_id, &file_path, file_type, now)?;
            Ok((existed, file.id))
        });
        match result {
            Ok((Some(_), Some(id))) => changed_ids.push(id),
            Ok((None, _)) => {
                added += 1;
                added_any = true;
            }
            Ok(_) => {}
            Err(e) => eprintln!("watch: failed to add {}: {}", file_path, e),
        }
    }
    if !added_any && changed_ids.is_empty() && removed == 0 {
        return;
    }
    if !changed_ids.is_empty() {
        // an overwritten photo needs a fresh thumbnail
        let _ = app.emit(
            "thumbnail_ready",
            serde_json::json!({
                "file_ids": changed_ids,
                "invalidate": true,
                "library_id": library_id,
                "metadata_changed": true,
            }),
        );
    }
    let _ = app.emit(
        "watch-folder-changed",
        WatchChanged {
            library_id: library_id.to_string(),
            album_id,
            folder_path: folder.to_string(),
            added,
            changed: changed_ids.len(),
            removed,
        },
    );
}

/// start watching `folder_path` (replacing any previous watch), or stop when `enabled` is false
#[tauri::command]
pub fn watch_folder(
    app_handle: AppHandle,
    album_id: i64,
    folder_path: String,
    enabled: bool,
) -> Result<(), String> {
    let mut guard = WATCHER.lock().map_err(|e| e.to_string())?;
    *guard = None; // dropping the old watcher ends its worker thread
    if !enabled || folder_path.is_empty() || !Path::new(&folder_path).is_dir() {
        return Ok(());
    }
    let library_id = crate::t_config::current_library_id()?;

    let (tx, rx) = mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            let _ = tx.send(path);
        }
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(Path::new(&folder_path), RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    let folder = folder_path.clone();
    std::thread::spawn(move || {
        // wait for the first event, then until the folder has been quiet for a moment
        while let Ok(first) = rx.recv() {
            let mut batch: HashSet<PathBuf> = HashSet::new();
            batch.insert(first);
            while let Ok(next) = rx.recv_timeout(Duration::from_millis(1200)) {
                batch.insert(next);
            }
            process(&app_handle, &library_id, album_id, &folder, batch);
        }
    });

    *guard = Some(watcher);
    Ok(())
}

/// Quick background check of one folder: compare the files on disk with the catalog and add
/// the new ones / drop the missing ones. Much cheaper than a full album scan.
#[tauri::command]
pub async fn sync_folder(app_handle: AppHandle, album_id: i64, folder_path: String) -> Result<(), String> {
    if folder_path.is_empty() || !Path::new(&folder_path).is_dir() {
        return Ok(());
    }
    let library_id = crate::t_config::current_library_id()?;
    tauri::async_runtime::spawn_blocking(move || {
        let on_disk: HashSet<String> = match std::fs::read_dir(&folder_path) {
            Ok(rd) => rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| t_utils::get_file_type(&Path::new(&folder_path).join(name).to_string_lossy()).is_some())
                .collect(),
            Err(_) => return,
        };
        let in_db = crate::t_cmds::with_current_library(&library_id, || {
            let folder_row = AFolder::add_to_db(album_id, &folder_path)?;
            let folder_id = folder_row.id.ok_or_else(|| "folder without id".to_string())?;
            AFile::list_names_in_folder(folder_id)
        });
        let Ok(in_db) = in_db else { return };
        let db_names: HashSet<&str> = in_db.iter().map(|(_, n)| n.as_str()).collect();
        let mut todo: HashSet<PathBuf> = HashSet::new();
        for name in &on_disk {
            if !db_names.contains(name.as_str()) {
                todo.insert(Path::new(&folder_path).join(name));
            }
        }
        for (_, name) in &in_db {
            if !on_disk.contains(name) {
                todo.insert(Path::new(&folder_path).join(name));
            }
        }
        if !todo.is_empty() {
            process(&app_handle, &library_id, album_id, &folder_path, todo);
        }
    })
    .await
    .map_err(|e| e.to_string())
}
