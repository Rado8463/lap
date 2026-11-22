<template>
  <div class="h-full flex flex-col overflow-hidden">
    <div class="px-2 h-8 shrink-0 flex items-center text-sm font-medium uppercase text-base-content/50 select-none">
      {{ titlebar }}
    </div>

    <div
      ref="listRef"
      tabindex="0"
      class="flex-1 min-h-0 overflow-auto outline-none pb-2"
      @keydown="handleKeyDown"
    >
      <div v-if="rows.length === 0" class="p-4 text-sm text-base-content/50">
        <span v-if="isLoadingRoots" class="loading loading-dots"></span>
        <span v-else>{{ $t('disk_tree.no_drives') }}</span>
      </div>

      <div
        v-for="row in rows"
        :key="row.node.path"
        :data-disk-path="row.node.path"
        class="group/folder h-8 flex items-center rounded-box whitespace-nowrap cursor-pointer border-2 border-transparent"
        :class="rowClass(row.node)"
        :style="{ paddingLeft: `${row.depth * 1}rem` }"
        @click="clickNode(row.node)"
        @dblclick="toggleNode(row.node)"
        @contextmenu.prevent.stop="openRowMenu(row.node, $event)"
      >
        <IconRight
          class="album-tree-chevron shrink-0"
          :class="[row.node.expanded ? 'rotate-90' : '', row.node.noChildren ? 'opacity-0 pointer-events-none' : '']"
          @click.stop="toggleNode(row.node)"
        />
        <span v-if="row.node.loading" class="loading loading-spinner loading-xs shrink-0 mx-1"></span>
        <component
          v-else
          :is="row.node.expanded ? IconFolderExpanded : IconFolder"
          class="p-1 w-6 h-6 shrink-0"
        />
        <span class="ml-1 truncate" :title="row.node.path">{{ row.node.name }}</span>
        <span v-if="row.node.albumId" class="ml-2 text-xs opacity-40 shrink-0">●</span>
      </div>
    </div>
    <ContextMenu ref="contextMenuRef" class="hidden" :menuItems="menuItems" />
  </div>
</template>

<script setup lang="ts">
// File-system browser for the left panel: drives and folders as they exist on
// disk. Choosing a folder shows its photos. Lap works on indexed albums, so a
// folder that is not part of any album yet gets a browse-only album (not listed
// under Albums; right-click "Add to Albums" promotes it).
// Albums cannot nest, so a folder that contains regular albums cannot be opened.
import { ref, computed, onMounted, reactive, nextTick, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { emit as tauriEmit } from '@tauri-apps/api/event';
import { config, libConfig } from '@/common/config';
import { isWin } from '@/common/utils';
import { getAllAlbums, addAlbum, editAlbum, removeAlbum, listAlbumSubfolders } from '@/common/api';
import ContextMenu from '@/components/ContextMenu.vue';
import { useAlbumSelectionProvider } from '@/composables/useAlbumSelection';
import { useToast } from '@/common/toast';
import { IconRight, IconFolder, IconFolderExpanded, IconOk } from '@/common/icons';

defineProps({
  titlebar: { type: String, default: '' },
});

interface DiskNode {
  name: string;
  path: string;
  isRoot: boolean;       // drive / filesystem root: browsable, but never added as an album
  expanded: boolean;
  loading: boolean;
  noChildren: boolean;   // known to have no sub-folders (or unreadable)
  children: string[] | null; // child paths once loaded
  albumId: number;       // id of the album that contains this folder, 0 if none
}

const { t } = useI18n();
const toast = useToast();
const selection = useAlbumSelectionProvider('album');

const SEP = isWin ? '\\' : '/';
const nodes = reactive(new Map<string, DiskNode>());
const rootPaths = ref<string[]>([]);
const isLoadingRoots = ref(true);
const albums = ref<any[]>([]);
const contextMenuRef = ref<any>(null);
const listRef = ref<HTMLElement | null>(null);

const normalize = (p: string) => (isWin ? p.toLowerCase() : p).replace(/[\\/]+$/, '');
const isInside = (child: string, parent: string) => {
  const c = normalize(child);
  const p = normalize(parent);
  return c === p || c.startsWith(p + SEP) || c.startsWith(p + '/');
};
const joinPath = (parent: string, name: string) =>
  parent.endsWith('\\') || parent.endsWith('/') ? parent + name : parent + SEP + name;

function getNode(path: string, name?: string, isRoot = false): DiskNode {
  let node = nodes.get(path);
  if (!node) {
    node = reactive({
      name: name ?? path,
      path,
      isRoot,
      expanded: false,
      loading: false,
      noChildren: false,
      children: null,
      albumId: 0,
    }) as DiskNode;
    nodes.set(path, node);
  }
  return node;
}

// Albums created only to browse a folder are kept in config.main.diskAlbumIds
// and not listed under Albums until the user adds them explicitly.
const isDiskAlbum = (id: number) => ((config.main as any).diskAlbumIds || []).map(Number).includes(Number(id));

function refreshAlbumMarks() {
  for (const node of nodes.values()) {
    const album = albums.value.find((a) => isInside(node.path, a.path) && !isDiskAlbum(a.id));
    node.albumId = album && !node.isRoot ? album.id : 0;
  }
}

async function loadRoots() {
  isLoadingRoots.value = true;
  try {
    if (isWin) {
      const letters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('');
      const probes = await Promise.allSettled(letters.map((l) => listAlbumSubfolders(`${l}:\\`)));
      rootPaths.value = letters.filter((_, i) => probes[i].status === 'fulfilled').map((l) => `${l}:\\`);
      for (const [i, letter] of letters.entries()) {
        const probe = probes[i];
        if (probe.status !== 'fulfilled') continue;
        const node = getNode(`${letter}:\\`, `${letter}:`, true);
        node.children = (probe.value as string[]).map((n) => joinPath(node.path, n));
        node.noChildren = node.children.length === 0;
        registerChildren(node);
      }
    } else {
      rootPaths.value = ['/'];
      getNode('/', '/', true);
    }
    albums.value = (await getAllAlbums()) || [];
    refreshAlbumMarks();
    await expandToCurrent();
  } finally {
    isLoadingRoots.value = false;
  }
}

function registerChildren(node: DiskNode) {
  for (const childPath of node.children || []) {
    const name = childPath.split(/[\\/]/).filter(Boolean).pop() || childPath;
    getNode(childPath, name);
  }
}

async function loadChildren(node: DiskNode) {
  if (node.children) return;
  node.loading = true;
  try {
    const names: string[] = (await listAlbumSubfolders(node.path)) || [];
    node.children = names.map((n) => joinPath(node.path, n));
    node.noChildren = node.children.length === 0;
    registerChildren(node);
    refreshAlbumMarks();
  } catch {
    // Unreadable folder (permissions, system folder, disconnected drive).
    node.children = [];
    node.noChildren = true;
  } finally {
    node.loading = false;
  }
}

async function toggleNode(node: DiskNode) {
  if (node.expanded) {
    node.expanded = false;
    return;
  }
  await loadChildren(node);
  node.expanded = !node.noChildren;
}

// Visible rows: depth-first walk over expanded nodes.
const rows = computed(() => {
  const out: { node: DiskNode; depth: number }[] = [];
  const walk = (path: string, depth: number) => {
    const node = nodes.get(path);
    if (!node) return;
    out.push({ node, depth });
    if (node.expanded) for (const child of node.children || []) walk(child, depth + 1);
  };
  for (const root of rootPaths.value) walk(root, 0);
  return out;
});

const currentPath = computed(() => String(libConfig.album.folderPath || ''));

function rowClass(node: DiskNode) {
  const selected = currentPath.value && normalize(currentPath.value) === normalize(node.path);
  return selected
    ? 'bg-base-100 text-base-content border-primary/40'
    : 'hover:bg-base-100/50';
}

async function clickNode(node: DiskNode) {
  listRef.value?.focus({ preventScroll: true });
  if (node.isRoot) {
    await toggleNode(node);
    return;
  }
  await openFolder(node);
}

// ---- context menu: promote a browse-only folder to a regular album --------
const menuNode = ref<DiskNode | null>(null);
const menuItems = computed(() => {
  const node = menuNode.value;
  const items: any[] = [];
  if (node && !node.isRoot) {
    const album = albums.value.find((a) => isInside(node.path, a.path));
    if (!album || isDiskAlbum(album.id)) {          // not in Albums yet
      items.push({ label: t('disk_tree.add_to_albums'), action: () => addToAlbums(node) });
      items.push({ label: t('disk_tree.add_to_albums_flat'), action: () => addToAlbumsFlat(node) });
      items.push({ label: '-', action: () => {} });
    }
  }
  items.push({
    label: t('settings.browse.show_subfolder_files'),
    icon: config.settings.showSubfolderFiles ? IconOk : undefined,
    iconClass: 'w-4 h-4',
    action: () => {
      const value = !config.settings.showSubfolderFiles;
      config.setShowSubfolderFiles(value);
      void tauriEmit('settings-showSubfolderFiles-changed', value);
    },
  });
  return items;
});

async function openRowMenu(node: DiskNode, event: MouseEvent) {
  albums.value = (await getAllAlbums()) || [];
  menuNode.value = node;
  await nextTick();
  contextMenuRef.value?.open?.(event.clientX, event.clientY);
}

async function addToAlbums(node: DiskNode) {
  albums.value = (await getAllAlbums()) || [];
  let album = albums.value.find((a) => isInside(node.path, a.path));
  if (!album) {
    await openFolder(node);                          // creates the browse-only album
    albums.value = (await getAllAlbums()) || [];
    album = albums.value.find((a) => isInside(node.path, a.path));
  }
  if (!album) return;
  config.main.diskAlbumIds = ((config.main as any).diskAlbumIds || []).filter((id: number) => Number(id) !== Number(album.id));
  await tauriEmit('albums-refreshed');
  refreshAlbumMarks();
  toast.success(t('disk_tree.added_to_albums', { name: album.name }));
}

// "This folder only": adds the folder as an album that excludes all its subfolders
// (the album exclusion works on first-level subfolders, with their whole content).
async function addToAlbumsFlat(node: DiskNode) {
  albums.value = (await getAllAlbums()) || [];
  const covering = albums.value.find((a) => isInside(node.path, a.path));
  if (covering && !isDiskAlbum(covering.id)) {
    toast.error(t('disk_tree.already_in_albums', { name: covering.name }));
    return;
  }
  const real = albums.value.find((a) => !isDiskAlbum(a.id) && isInside(a.path, node.path));
  if (real) {
    toast.error(t('disk_tree.overlap', { name: real.name }));
    return;
  }
  const filters = { fileTypes: 7, smallImageFilter: 0, excludedFolders: (await listAlbumSubfolders(node.path)) || [] };
  let album: any = null;
  if (covering && normalize(covering.path) === normalize(node.path)) {
    await editAlbum(covering.id, covering.name, covering.description || '', filters);
    album = covering;
  } else {
    // Browse-only albums overlapping the folder are replaced by the new album.
    for (const old of albums.value.filter((a) => isDiskAlbum(a.id) && (isInside(node.path, a.path) || isInside(a.path, node.path)))) {
      await removeAlbum(old.id);
      config.main.diskAlbumIds = ((config.main as any).diskAlbumIds || []).filter((id: number) => Number(id) !== Number(old.id));
    }
    album = await addAlbum(node.path, node.name || node.path, '', filters);
  }
  if (!album) {
    toast.error(t('disk_tree.add_failed'));
    return;
  }
  config.main.diskAlbumIds = ((config.main as any).diskAlbumIds || []).filter((id: number) => Number(id) !== Number(album.id));
  albums.value = (await getAllAlbums()) || [];
  await tauriEmit('albums-refreshed');
  await tauriEmit('library-total-refreshed');
  if (libConfig.index.albumQueue.indexOf(album.id) === -1) libConfig.index.albumQueue.push(album.id);
  libConfig.index.status = 1;
  refreshAlbumMarks();
  await selection.selectFolder(album.id, { id: 0, name: node.name, path: node.path } as any);
  toast.success(t('disk_tree.added_to_albums', { name: album.name }));
}

// Folders only browsed in the Disk tab are indexed as hidden browse-only albums. Unless the user
// wants to keep their data and thumbnails, everything except the folder on screen is removed.
async function cleanupDiskAlbums(keepPath: string) {
  if (config.settings.diskKeepBrowseCache === true) return;
  const ids: number[] = ((config.main as any).diskAlbumIds || []).map(Number);
  if (!ids.length) return;
  const all = (await getAllAlbums()) || [];
  const kept: number[] = [];
  let removed = false;
  for (const id of ids) {
    const album = all.find((a) => Number(a.id) === id);
    if (album && keepPath && isInside(keepPath, album.path)) { kept.push(id); continue; }
    if (album) { await removeAlbum(id); removed = true; }
  }
  (config.main as any).diskAlbumIds = kept;
  if (removed) {
    albums.value = (await getAllAlbums()) || [];
    await tauriEmit('albums-refreshed');
    await tauriEmit('library-total-refreshed');
    refreshAlbumMarks();
  }
}

async function openFolder(node: DiskNode) {
  albums.value = (await getAllAlbums()) || [];

  // 1. already inside an album: just select it there
  let album = albums.value.find((a) => isInside(node.path, a.path));

  // A subfolder the album excludes ("this folder only") was never indexed and would look empty:
  // drop the exclusion so it gets indexed. (Albums cannot nest, so it cannot be its own album.)
  if (album && Array.isArray(album.excluded_folders) && album.excluded_folders.length > 0
      && normalize(node.path) !== normalize(album.path)) {
    const first = node.path.slice(album.path.length).split(/[\\/]/).filter(Boolean)[0];
    if (first && album.excluded_folders.includes(first)) {
      const remaining = album.excluded_folders.filter((f: string) => f !== first);
      await editAlbum(album.id, album.name, album.description || '', {
        fileTypes: album.file_types ?? 7, smallImageFilter: album.small_image_filter ?? 0, excludedFolders: remaining,
      });
      album.excluded_folders = remaining;
      if (libConfig.index.albumQueue.indexOf(album.id) === -1) libConfig.index.albumQueue.push(album.id);
      libConfig.index.status = 1;
      await tauriEmit('albums-refreshed');
      toast.success(t('disk_tree.exclusion_removed', { folder: first, name: album.name }));
    }
  }

  // 2. not covered by an album: add it as a *browse-only* album (not listed
  //    under Albums). Albums cannot nest, so browse-only albums inside the new
  //    folder are merged into it; real albums inside it block the action.
  if (!album) {
    const contained = albums.value.filter((a) => isInside(a.path, node.path));
    const real = contained.find((a) => !isDiskAlbum(a.id));
    if (real) {
      toast.error(t('disk_tree.overlap', { name: real.name }));
      return;
    }
    for (const old of contained) {
      await removeAlbum(old.id);
      config.main.diskAlbumIds = ((config.main as any).diskAlbumIds || []).filter((id: number) => Number(id) !== Number(old.id));
    }
    if (contained.length > 0) albums.value = albums.value.filter((a) => !contained.some((c) => c.id === a.id));
    const name = node.name || node.path;
    album = await addAlbum(node.path, name, '', { fileTypes: 7, smallImageFilter: 0, excludedFolders: [] });
    if (!album) {
      toast.error(t('disk_tree.add_failed'));
      return;
    }
    (config.main as any).diskAlbumIds = [...((config.main as any).diskAlbumIds || []), album.id];
    albums.value.push(album);
    await tauriEmit('albums-refreshed');
    await tauriEmit('library-total-refreshed');
    // queue indexing, same as the album list does after adding an album
    if (libConfig.index.albumQueue.indexOf(album.id) === -1) libConfig.index.albumQueue.push(album.id);
    libConfig.index.status = 1;
  }
  refreshAlbumMarks();

  // Always select the folder itself (never the whole album) so a root folder
  // shows only its own files unless "Show files from subfolders" is on.
  await selection.selectFolder(album.id, { id: 0, name: node.name, path: node.path } as any);
  void cleanupDiskAlbums(node.path);
}

// Expand the tree down to the currently selected album folder.
async function expandToCurrent() {
  const target = currentPath.value;
  if (!target) return;
  const root = rootPaths.value.find((r) => isInside(target, r));
  if (!root) return;
  let node = getNode(root);
  if (!node.expanded) await toggleNode(node);
  const rest = target.slice(root.length).split(/[\\/]/).filter(Boolean);
  let path = root;
  for (const part of rest) {
    path = joinPath(path, part);
    const child = nodes.get(path);
    if (!child) return;
    if (part !== rest[rest.length - 1] && !child.expanded) await toggleNode(child);
    node = child;
  }
}

async function handleKeyDown(event: KeyboardEvent) {
  const list = rows.value;
  const index = list.findIndex((r) => normalize(r.node.path) === normalize(currentPath.value));
  const move = async (delta: number) => {
    const next = list[Math.min(Math.max(index + delta, 0), list.length - 1)];
    if (next) {
      event.preventDefault();
      await clickNode(next.node);
      listRef.value?.querySelector(`[data-disk-path="${CSS.escape(next.node.path)}"]`)?.scrollIntoView({ block: 'nearest' });
    }
  };
  if (event.key === 'ArrowDown') await move(1);
  else if (event.key === 'ArrowUp') await move(-1);
  else if (event.key === 'ArrowRight' && index >= 0) {
    event.preventDefault();
    const n = list[index].node;
    if (!n.expanded) await toggleNode(n);
  } else if (event.key === 'ArrowLeft' && index >= 0) {
    event.preventDefault();
    const n = list[index].node;
    if (n.expanded) n.expanded = false;
  }
}

// Follow the selection when it changes from outside (e.g. folder history shortcuts).
watch(currentPath, () => {
  void expandToCurrent().then(() => nextTick(() => {
    const path = currentPath.value;
    if (path) listRef.value?.querySelector(`[data-disk-path="${CSS.escape(path)}"]`)?.scrollIntoView({ block: 'nearest' });
  }));
});

onMounted(async () => {
  await loadRoots();
  await cleanupDiskAlbums(currentPath.value);
});
</script>
