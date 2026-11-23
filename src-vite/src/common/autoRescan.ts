import { config, libConfig } from '@/common/config';
import { isWin } from '@/common/utils';
import { invoke } from '@tauri-apps/api/core';

// Lap watches a folder only when the user turned that on for it. Otherwise, opening an album or a
// folder runs a quick background check (files on disk vs. catalog, no thumbnails, no full scan) that
// adds new and drops missing files; the view refreshes by itself if something changed.
const MIN_INTERVAL_MS = 15_000;
const lastCheck = new Map<string, number>();

export function autoRescanAlbum(albumId: number, folderPath: string) {
  if (config.settings.autoRescanOnOpen === false) return;
  const id = Number(albumId);
  if (!(id > 0) || !folderPath) return;
  const key = `${id}|${folderPath}`;
  const now = Date.now();
  if (now - (lastCheck.get(key) || 0) < MIN_INTERVAL_MS) return;
  lastCheck.set(key, now);
  void invoke('sync_folder', { albumId: id, folderPath }).catch(() => {});
}

const norm = (p: string) => (isWin ? p.toLowerCase() : p).replace(/[\\/]+$/, '');
export const samePath = (a: string, b: string) => norm(String(a || '')) === norm(String(b || ''));
export const isWatchedFolder = (path: string) =>
  (((config.main as any).watchedFolders || []) as string[]).some(p => samePath(p, path));
