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
  </div>
</template>

<script setup lang="ts">
// File-system browser for the left panel: drives and folders as they exist on
// disk. Choosing a folder shows its photos. Lap works on indexed albums, so a
// folder that is not part of any album yet is added as an album automatically.
// Albums cannot nest, so a folder that contains existing albums cannot be added.
import { ref, computed, onMounted, reactive } from 'vue';
import { useI18n } from 'vue-i18n';
import { emit as tauriEmit } from '@tauri-apps/api/event';
import { libConfig } from '@/common/config';
import { isWin } from '@/common/utils';
import { getAllAlbums, addAlbum, listAlbumSubfolders } from '@/common/api';
import { useAlbumSelectionProvider } from '@/composables/useAlbumSelection';
import { useToast } from '@/common/toast';
import { IconRight, IconFolder, IconFolderExpanded } from '@/common/icons';

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

function refreshAlbumMarks() {
  for (const node of nodes.values()) {
    const album = albums.value.find((a) => isInside(node.path, a.path));
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

async function openFolder(node: DiskNode) {
  albums.value = (await getAllAlbums()) || [];

  // 1. already inside an album: just select it there
  let album = albums.value.find((a) => isInside(node.path, a.path));

  // 2. not covered by an album: add it (unless it would overlap existing albums)
  if (!album) {
    const contained = albums.value.filter((a) => isInside(a.path, node.path));
    if (contained.length > 0) {
      toast.error(t('disk_tree.overlap', { name: contained[0].name }));
      return;
    }
    const name = node.name || node.path;
    album = await addAlbum(node.path, name, '', { fileTypes: 7, smallImageFilter: 0, excludedFolders: [] });
    if (!album) {
      toast.error(t('disk_tree.add_failed'));
      return;
    }
    albums.value.push(album);
    await tauriEmit('albums-refreshed');
    await tauriEmit('library-total-refreshed');
    // queue indexing, same as the album list does after adding an album
    if (libConfig.index.albumQueue.indexOf(album.id) === -1) libConfig.index.albumQueue.push(album.id);
    libConfig.index.status = 1;
    toast.success(t('disk_tree.added', { name }));
  }
  refreshAlbumMarks();

  if (normalize(album.path) === normalize(node.path)) {
    selection.selectAlbum(album);
  } else {
    await selection.selectFolder(album.id, { id: 0, name: node.name, path: node.path } as any);
  }
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

onMounted(loadRoots);
</script>
