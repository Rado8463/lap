// Albums that only exist to browse a folder from the Disk tab are temporary: they are removed at
// startup and when Lap closes (unless the user chose to keep the browse data).
import { config, libConfig } from '@/common/config';
import { getAllAlbums, removeAlbum } from '@/common/api';

type MainConfig = { diskAlbumIds?: number[]; diskAlbumPaths?: Record<string, string>; diskAlbumLibraryId?: string };

const main = () => config.main as unknown as MainConfig;
const norm = (p: string) => String(p || '').replace(/[\\/]+$/, '').toLowerCase();

// Remembers the path (and library) of a browse-only album, so a stale id can never remove another album.
export function registerDiskAlbum(album: { id: number; path: string }) {
  const m = main();
  m.diskAlbumPaths = { ...(m.diskAlbumPaths || {}), [String(album.id)]: album.path };
  m.diskAlbumLibraryId = String((libConfig as any)._libraryId ?? '');
}

const inside = (child: string, parent: string) => {
  const c = norm(child).replace(/\\/g, '/');
  const p = norm(parent).replace(/\\/g, '/');
  return !!p && (c === p || c.startsWith(p + '/'));
};

// `keepPath`: the folder shown on start in the Disk tab; its album is still in use and stays.
export async function cleanupAllDiskAlbums(keepPath = ''): Promise<void> {
  if ((config.settings as any).diskKeepBrowseCache === true) return;
  const m = main();
  const ids = (m.diskAlbumIds || []).map(Number);
  if (!ids.length) return;
  const libraryId = String((libConfig as any)._libraryId ?? '');
  // The ids belong to the library that was open when they were made.
  if (m.diskAlbumLibraryId && libraryId && m.diskAlbumLibraryId !== libraryId) return;
  try {
    const all = (await getAllAlbums()) || [];
    const kept: number[] = [];
    for (const id of ids) {
      const album = all.find((a: any) => Number(a.id) === id);
      if (album && keepPath && inside(keepPath, album.path)) { kept.push(id); continue; }
      // Ids made before the paths were recorded are trusted as they are.
      const path = (m.diskAlbumPaths || {})[String(id)] ?? album?.path;
      if (album && path && norm(album.path) === norm(path)) await removeAlbum(id);
    }
    // Only what was handled here: an album registered meanwhile (opened at startup) stays.
    const left = [...kept, ...(m.diskAlbumIds || []).map(Number).filter((id) => !ids.includes(id))];
    m.diskAlbumIds = left;
    const paths = { ...(m.diskAlbumPaths || {}) };
    for (const id of ids) if (!kept.includes(id)) delete paths[String(id)];
    m.diskAlbumPaths = paths;
  } catch (error) {
    console.error('[diskAlbums] cleanup failed:', error);
  }
}
