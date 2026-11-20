// Per-file saved viewport (zoom level + visible position), similar to the
// "save viewport" feature of Pictureflect Photo Viewer.
//
// Stored in localStorage (shared by the main window and the separate viewer
// window, which run on the same origin). Keyed by absolute file path.

const STORAGE_KEY = 'lap.savedViewports.v1';
const MAX_ENTRIES = 20000;

export interface SavedViewport {
  /** Zoom in physical screen pixels per image pixel (1 = 100 %). Independent of devicePixelRatio. */
  displayScale: number;
  /** Image-content point at the viewport centre, normalised 0..1 (unrotated image space). */
  normX: number;
  normY: number;
  /** Layout width of the image the viewport was captured on (guards against dimension changes). */
  sourceWidth: number;
  savedAt: number;
}

type Store = Record<string, SavedViewport>;

let cache: Store | null = null;

function load(): Store {
  if (cache) return cache;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    cache = raw ? (JSON.parse(raw) as Store) : {};
  } catch {
    cache = {};
  }
  return cache;
}

function persist(store: Store) {
  try {
    const keys = Object.keys(store);
    if (keys.length > MAX_ENTRIES) {
      // Drop the oldest entries.
      keys
        .sort((a, b) => (store[a].savedAt || 0) - (store[b].savedAt || 0))
        .slice(0, keys.length - MAX_ENTRIES)
        .forEach((k) => delete store[k]);
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch (e) {
    console.error('Failed to persist saved viewports:', e);
  }
}

// Another window may have written since we cached.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    if (event.key === STORAGE_KEY) cache = null;
  });
}

export function getSavedViewport(filePath?: string | null): SavedViewport | null {
  if (!filePath) return null;
  const entry = load()[filePath];
  if (!entry || !(entry.displayScale > 0)) return null;
  return entry;
}

export function setSavedViewport(filePath: string, viewport: Omit<SavedViewport, 'savedAt'>) {
  const store = load();
  store[filePath] = { ...viewport, savedAt: Date.now() };
  persist(store);
}

export function removeSavedViewport(filePath?: string | null): boolean {
  if (!filePath) return false;
  const store = load();
  if (!(filePath in store)) return false;
  delete store[filePath];
  persist(store);
  return true;
}
