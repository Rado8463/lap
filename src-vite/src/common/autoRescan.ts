import { config, libConfig } from '@/common/config';

// Lap has no folder watcher, so photos added to a folder from outside (Photoshop exports,
// renders) appear only after a scan. Opening an album or one of its folders queues a scan,
// at most once per album per interval, unless the album is already queued/scanning.
const MIN_INTERVAL_MS = 30_000;
const lastAutoScan = new Map<number, number>();

export function autoRescanAlbum(albumId: number) {
  if (config.settings.autoRescanOnOpen === false) return;
  const id = Number(albumId);
  if (!(id > 0)) return;
  const queue = libConfig.index.albumQueue as any[];
  if (queue.some(q => Number(q) === id)) return;
  if (((libConfig.index.pausedAlbumIds as any[]) || []).some(q => Number(q) === id)) return;
  if (Number(libConfig.index.status || 0) === 2) return; // recovery dialog pending
  const now = Date.now();
  if (now - (lastAutoScan.get(id) || 0) < MIN_INTERVAL_MS) return;
  lastAutoScan.set(id, now);
  queue.push(id);
  libConfig.index.status = 1;
}
