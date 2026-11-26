<template>
  <ModalDialog :title="batch ? `${$t('convert.title')} – ${$t('convert.batch_title', { count: files.length })}` : `${$t('convert.title')} – ${fileName}`" :width="dialogWidth" :height="dialogHeight" @cancel="close">
    <div class="flex flex-1 min-h-0 gap-3 text-sm">
      <!-- comparison -->
      <div class="flex-1 min-w-0 flex flex-col gap-2">
        <div class="flex items-center gap-3 text-xs shrink-0">
          <button class="t-button-default" :disabled="!ready" @click="toggleZoom">
            {{ zoomed ? $t('convert.fit') : $t('convert.actual_size') }}
          </button>
          <span class="text-base-content/50 truncate">{{ $t(zoomed ? 'convert.hint_zoomed' : 'convert.hint_fit') }}</span>
          <template v-if="batch">
            <span class="ml-auto shrink-0 text-base-content/50">{{ $t('convert.sample') }}</span>
            <select v-model.number="sampleIndex" class="select select-bordered select-xs w-56 text-xs" :disabled="converting">
              <option v-for="(file, index) in files" :key="index" :value="index">{{ file.fileName }}</option>
            </select>
          </template>
        </div>
        <div class="flex flex-1 min-h-0 gap-2">
          <div v-for="side in ['original', 'converted']" :key="side" class="flex-1 min-w-0 flex flex-col gap-1">
            <div class="text-xs font-bold uppercase tracking-widest text-base-content/40 shrink-0">
              {{ $t(side === 'original' ? 'convert.original' : 'convert.result') }}
            </div>
            <div
              :ref="el => setPaneRef(side, el)"
              class="relative flex-1 min-h-0 overflow-hidden rounded-box bg-base-300/60 border border-base-content/10 select-none"
              :class="zoomed ? 'cursor-grab' : 'cursor-zoom-in'"
              @mousedown="onPaneMouseDown($event, side)"
              @dblclick="zoomed && toggleZoom()"
            >
              <img
                v-if="!zoomed && fitSrc(side)"
                :src="fitSrc(side)"
                class="absolute inset-0 w-full h-full object-contain pointer-events-none"
                draggable="false"
              />
              <img
                v-else-if="zoomed && regionSrc(side)"
                :src="regionSrc(side)"
                class="absolute pointer-events-none max-w-none"
                :style="regionStyle(side)"
                draggable="false"
              />
              <div v-if="busy && side === 'converted'" class="absolute inset-0 flex items-center justify-center bg-base-300/40 pointer-events-none">
                <span class="loading loading-spinner loading-md"></span>
              </div>
              <div v-if="errorText && side === 'converted'" class="absolute inset-x-2 bottom-2 text-xs text-error bg-base-300/80 rounded p-1">
                {{ errorText }}
              </div>
            </div>
            <div class="text-xs text-base-content/70 shrink-0 min-h-[2.5rem]">
              <div v-if="info">{{ $t('convert.dimensions') }}: {{ info.width }} × {{ info.height }} × 24</div>
              <div v-if="side === 'original' && info">
                {{ $t('convert.file') }}: {{ fileName }}, {{ formatBytes(info.originalBytes) }}
              </div>
              <div v-else-if="side === 'converted' && resultBytes !== null">
                {{ $t('convert.file') }}: {{ resultName }}, {{ formatBytes(resultBytes) }}
                <span v-if="savedPercent !== null" :class="savedPercent >= 0 ? 'text-success' : 'text-warning'">
                  ({{ savedPercent >= 0 ? '−' : '+' }}{{ Math.abs(savedPercent).toFixed(0) }} %)
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- settings -->
      <div class="w-64 shrink-0 flex flex-col gap-3">
        <div class="font-bold">{{ $t('convert.format') }}</div>
        <div class="rounded-box border border-base-content/10 bg-base-300/30 px-2 py-1 text-xs">JPEG Image (*.jpg)</div>

        <div>
          <div class="flex items-center justify-between mb-1">
            <span>{{ $t('convert.quality') }}</span>
            <span class="tabular-nums">{{ options.quality }}</span>
          </div>
          <input type="range" min="1" max="100" step="1" class="range range-xs w-full [--range-fill:0]" v-model.number="options.quality" />
        </div>

        <div>
          <div class="mb-1">{{ $t('convert.sampling') }}</div>
          <select v-model.number="options.sampling" class="select select-bordered select-sm w-full text-xs">
            <option :value="0">1:1 (4:4:4)</option>
            <option :value="1">4:2:2</option>
            <option :value="2">4:2:0</option>
          </select>
        </div>

        <label class="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" class="checkbox checkbox-primary checkbox-xs" v-model="options.progressive" />
          <span>{{ $t('convert.progressive') }}</span>
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" class="checkbox checkbox-primary checkbox-xs" v-model="options.optimize" :disabled="options.progressive" />
          <span>{{ $t('convert.optimize') }}</span>
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" class="checkbox checkbox-primary checkbox-xs" v-model="options.restartMarkers" />
          <span>{{ $t('convert.restart_markers') }}</span>
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" class="checkbox checkbox-primary checkbox-xs" v-model="options.keepMetadata" />
          <span>{{ $t('convert.keep_metadata') }}</span>
        </label>

        <div>
          <div class="mb-1">{{ $t('convert.transparency') }}</div>
          <input type="color" class="h-8 w-14 cursor-pointer rounded-box border border-base-content/20 bg-transparent p-0.5" v-model="background" />
        </div>

        <div class="mt-auto flex flex-col gap-2">
          <div v-if="converting" class="space-y-1 text-xs">
            <div class="flex justify-between"><span>{{ $t('convert.converting') }}</span><span class="tabular-nums">{{ done }} / {{ files.length }}</span></div>
            <progress class="progress progress-primary w-full" :value="done" :max="files.length"></progress>
          </div>
          <div v-if="saveError" class="text-xs text-error">{{ saveError }}</div>
          <button v-if="!converting" class="t-button-primary" :disabled="!ready || busy || saving" @click="apply">
            {{ batch ? $t('convert.apply_batch', { count: files.length }) : $t('convert.apply') }}
          </button>
          <button class="t-button-default" :disabled="saving && !converting" @click="close">{{ $t('msgbox.cancel') }}</button>
        </div>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from 'vue-i18n';
import { useUIStore } from '@/stores/uiStore';
import ModalDialog from '@/components/ModalDialog.vue';

type OpenResult = { width: number; height: number; originalBytes: number; originalPreview: string };
type Region = { image: string; x: number; y: number; width: number; height: number };

type ConvertFile = { filePath: string; fileName: string; folderId: number };
type BatchResult = { saved: { path: string; folderId: number }[]; failed: number };
const props = defineProps<{ files: ConvertFile[] }>();
const emit = defineEmits<{ (e: 'close'): void; (e: 'saved', path: string): void; (e: 'batch-done', result: BatchResult): void }>();
const { t } = useI18n();
const uiStore = useUIStore();

// Batch: one photo (the sample) drives the preview, the settings are applied to every file.
const batch = computed(() => props.files.length > 1);
const sampleIndex = ref(0);
const filePath = computed(() => props.files[sampleIndex.value]?.filePath || '');
const fileName = computed(() => props.files[sampleIndex.value]?.fileName || '');
const converting = ref(false);
const done = ref(0);
let stopRequested = false;

const STORAGE_KEY = 'lap.convertDialog.v1';
const DEFAULTS = { quality: 90, sampling: 0, progressive: false, optimize: true, restartMarkers: false, keepMetadata: true, background: '#ffffff' };

function loadSaved() {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}') || {};
    return {
      quality: Math.min(100, Math.max(1, Number(saved.quality) || DEFAULTS.quality)),
      sampling: [0, 1, 2].includes(Number(saved.sampling)) ? Number(saved.sampling) : DEFAULTS.sampling,
      progressive: saved.progressive === true, // off unless the user turned it on
      optimize: saved.optimize !== false,
      restartMarkers: saved.restartMarkers === true, // off unless the user turned it on
      keepMetadata: saved.keepMetadata !== false,
      background: /^#[0-9a-fA-F]{6}$/.test(saved.background) ? saved.background : DEFAULTS.background,
    };
  } catch {
    return { ...DEFAULTS };
  }
}
const saved = loadSaved();
const options = reactive({
  quality: saved.quality,
  sampling: saved.sampling,
  progressive: saved.progressive,
  optimize: saved.optimize,
  restartMarkers: saved.restartMarkers,
  keepMetadata: saved.keepMetadata,
});
const background = ref(saved.background);

const dialogWidth = Math.min(1240, Math.max(760, window.innerWidth - 60));
const dialogHeight = Math.min(780, Math.max(520, window.innerHeight - 60));

const info = ref<OpenResult | null>(null);
const resultBytes = ref<number | null>(null);
const resultPreview = ref('');
const busy = ref(false);
const saving = ref(false);
const errorText = ref('');
const saveError = ref('');
const ready = computed(() => !!info.value && !errorText.value);

const resultName = computed(() => fileName.value.replace(/\.[^./\\]+$/, '') + '.jpg');
const savedPercent = computed(() => {
  if (!info.value || resultBytes.value === null || info.value.originalBytes <= 0) return null;
  return (1 - resultBytes.value / info.value.originalBytes) * 100;
});

function formatBytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  const text = mb >= 1 ? `${mb.toFixed(1)} MB` : `${(bytes / 1024).toFixed(1)} KB`;
  return `${text} (${bytes.toLocaleString('sk-SK').replace(/ /g, ' ')} bytes)`;
}

function hexToRgb(hex: string): number {
  return parseInt(hex.replace('#', ''), 16) || 0;
}

function currentOptions() {
  return {
    quality: options.quality,
    sampling: options.sampling,
    progressive: options.progressive,
    optimize: options.optimize || options.progressive,
    restartMarkers: options.restartMarkers,
    keepMetadata: options.keepMetadata,
  };
}

// ---- loading and previews -------------------------------------------------
let openSeq = 0;
let previewSeq = 0;
let previewTimer: ReturnType<typeof setTimeout> | null = null;
let disposed = false;

async function openPicture() {
  const seq = ++openSeq;
  busy.value = true;
  errorText.value = '';
  resultBytes.value = null;
  resultPreview.value = '';
  try {
    const result = await invoke<OpenResult>('convert_open', { filePath: filePath.value, background: hexToRgb(background.value) });
    if (disposed || seq !== openSeq) return;
    info.value = result;
    await runPreview();
  } catch (error) {
    if (disposed || seq !== openSeq) return;
    console.error('convert_open failed:', error);
    info.value = null;
    errorText.value = String(error);
    busy.value = false;
  }
}

async function runPreview() {
  const seq = ++previewSeq;
  busy.value = true;
  try {
    const result = await invoke<{ bytes: number; preview: string }>('convert_preview', {
      filePath: filePath.value,
      options: currentOptions(),
    });
    if (disposed || seq !== previewSeq) return;
    resultBytes.value = result.bytes;
    resultPreview.value = result.preview;
    errorText.value = '';
    if (zoomed.value) void fetchRegions();
  } catch (error) {
    if (disposed || seq !== previewSeq) return;
    console.error('convert_preview failed:', error);
    errorText.value = String(error);
  } finally {
    if (!disposed && seq === previewSeq) busy.value = false;
  }
}

function schedulePreview() {
  if (!info.value) return;
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = setTimeout(() => { previewTimer = null; void runPreview(); }, 350);
}

watch(() => [options.quality, options.sampling, options.progressive, options.optimize, options.restartMarkers], schedulePreview);
watch(sampleIndex, () => {
  zoomed.value = false;
  regions.original = null;
  regions.converted = null;
  center.x = 0;
  center.y = 0;
  void openPicture();
});
let backgroundTimer: ReturnType<typeof setTimeout> | null = null;
watch(background, () => {
  if (backgroundTimer) clearTimeout(backgroundTimer);
  backgroundTimer = setTimeout(() => { backgroundTimer = null; void openPicture(); }, 400);
});

// ---- panes: fitted pictures and 1:1 crops -----------------------------------
const panes: Record<string, HTMLElement | null> = { original: null, converted: null };
function setPaneRef(side: string, el: any) { panes[side] = (el as HTMLElement) || null; }

function fitSrc(side: string): string {
  return side === 'original' ? (info.value?.originalPreview || '') : resultPreview.value;
}

const zoomed = ref(false);
const center = reactive({ x: 0, y: 0 });
const regions = reactive<{ original: Region | null; converted: Region | null }>({ original: null, converted: null });
let regionSeq = 0;
let regionBusy = false;
let regionDirty = false;

function regionSrc(side: string): string {
  return (regions as any)[side]?.image || '';
}

// Position of a crop inside its pane: the pane shows the picture at 1:1 around `center`.
function regionStyle(side: string) {
  const region: Region | null = (regions as any)[side];
  const pane = panes[side];
  if (!region || !pane) return {};
  // 1:1 means one picture pixel per screen pixel, whatever the Windows scaling is.
  const s = 1 / (window.devicePixelRatio || 1);
  const left = pane.clientWidth / 2 - (center.x - region.x) * s;
  const top = pane.clientHeight / 2 - (center.y - region.y) * s;
  return { left: `${left}px`, top: `${top}px`, width: `${region.width * s}px`, height: `${region.height * s}px` };
}

async function fetchRegions() {
  if (!zoomed.value || !info.value) return;
  if (regionBusy) { regionDirty = true; return; }
  regionBusy = true;
  const seq = ++regionSeq;
  const pane = panes.original || panes.converted;
  // A little more than the pane, so a small drag never shows an empty edge.
  const dpr = window.devicePixelRatio || 1;
  const w = Math.min(3000, Math.ceil((pane?.clientWidth || 600) * dpr * 1.5));
  const h = Math.min(3000, Math.ceil((pane?.clientHeight || 450) * dpr * 1.5));
  try {
    const cx = Math.round(center.x);
    const cy = Math.round(center.y);
    const [original, converted] = await Promise.all([
      invoke<Region>('convert_region', { filePath: filePath.value, converted: false, cx, cy, w, h }),
      resultBytes.value !== null
        ? invoke<Region>('convert_region', { filePath: filePath.value, converted: true, cx, cy, w, h })
        : Promise.resolve(null),
    ]);
    if (disposed || seq !== regionSeq) return;
    regions.original = original;
    regions.converted = converted;
  } catch (error) {
    console.error('convert_region failed:', error);
  } finally {
    regionBusy = false;
    if (regionDirty && !disposed) { regionDirty = false; void fetchRegions(); }
  }
}

function toggleZoom() {
  if (zoomed.value) {
    zoomed.value = false;
    regions.original = null;
    regions.converted = null;
    return;
  }
  if (!info.value) return;
  if (center.x === 0 && center.y === 0) { center.x = info.value.width / 2; center.y = info.value.height / 2; }
  zoomed.value = true;
  void fetchRegions();
}

let drag: { startX: number; startY: number; cx: number; cy: number; moved: boolean; side: string } | null = null;

function onPaneMouseDown(event: MouseEvent, side: string) {
  if (event.button !== 0 || !info.value) return;
  drag = { startX: event.clientX, startY: event.clientY, cx: center.x, cy: center.y, moved: false, side };
  window.addEventListener('mousemove', onDragMove);
  window.addEventListener('mouseup', onDragEnd);
}

function onDragMove(event: MouseEvent) {
  if (!drag || !zoomed.value) return;
  const dx = event.clientX - drag.startX;
  const dy = event.clientY - drag.startY;
  if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
  const maxX = info.value?.width ?? 0;
  const maxY = info.value?.height ?? 0;
  const dpr = window.devicePixelRatio || 1;
  center.x = Math.min(maxX, Math.max(0, drag.cx - dx * dpr));
  center.y = Math.min(maxY, Math.max(0, drag.cy - dy * dpr));
  void fetchRegions();
}

function onDragEnd(event: MouseEvent) {
  window.removeEventListener('mousemove', onDragMove);
  window.removeEventListener('mouseup', onDragEnd);
  const current = drag;
  drag = null;
  if (!current || current.moved || zoomed.value || !info.value) return;
  // A click on a fitted picture zooms to 100 % around the clicked point.
  const pane = panes[current.side];
  if (!pane) return;
  const rect = pane.getBoundingClientRect();
  const scale = Math.min(rect.width / info.value.width, rect.height / info.value.height);
  const shownW = info.value.width * scale;
  const shownH = info.value.height * scale;
  const px = (event.clientX - rect.left - (rect.width - shownW) / 2) / scale;
  const py = (event.clientY - rect.top - (rect.height - shownH) / 2) / scale;
  center.x = Math.min(info.value.width, Math.max(0, px));
  center.y = Math.min(info.value.height, Math.max(0, py));
  zoomed.value = true;
  void fetchRegions();
}

// ---- apply / close ------------------------------------------------------------
function rememberSettings() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...options, background: background.value }));
  } catch { /* storage unavailable: settings are simply not remembered */ }
}

async function applyBatch() {
  const result: BatchResult = { saved: [], failed: 0 };
  converting.value = true;
  stopRequested = false;
  done.value = 0;
  const bg = hexToRgb(background.value);
  for (const file of props.files) {
    if (stopRequested || disposed) break;
    try {
      const path = await invoke<string>('convert_file', { filePath: file.filePath, options: currentOptions(), background: bg });
      result.saved.push({ path, folderId: file.folderId });
    } catch (error) {
      console.error('convert_file failed:', file.filePath, error);
      result.failed++;
    }
    done.value++;
  }
  converting.value = false;
  rememberSettings();
  emit('batch-done', result);
  emit('close');
}

async function apply() {
  if (!ready.value || saving.value) return;
  saving.value = true;
  saveError.value = '';
  if (batch.value) {
    try {
      if (previewTimer) { clearTimeout(previewTimer); previewTimer = null; }
      await applyBatch();
    } finally {
      saving.value = false;
    }
    return;
  }
  try {
    if (previewTimer) { clearTimeout(previewTimer); previewTimer = null; }
    const path = await invoke<string>('convert_save', { filePath: filePath.value, options: currentOptions() });
    rememberSettings();
    emit('saved', path);
    emit('close');
  } catch (error) {
    console.error('convert_save failed:', error);
    saveError.value = `${t('convert.save_failed')}: ${String(error)}`;
  } finally {
    saving.value = false;
  }
}

function close() {
  if (converting.value) {
    stopRequested = true; // finish the current file, convert no more
    return;
  }
  if (saving.value) return;
  rememberSettings();
  emit('close');
}

function handleKeyDown(event: KeyboardEvent) {
  if (!uiStore.isInputActive('ConvertDialog')) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    close();
  }
}

onMounted(() => {
  uiStore.pushInputHandler('ConvertDialog');
  window.addEventListener('keydown', handleKeyDown, true);
  void openPicture();
});

onBeforeUnmount(() => {
  disposed = true;
  if (previewTimer) clearTimeout(previewTimer);
  if (backgroundTimer) clearTimeout(backgroundTimer);
  window.removeEventListener('keydown', handleKeyDown, true);
  window.removeEventListener('mousemove', onDragMove);
  window.removeEventListener('mouseup', onDragEnd);
  uiStore.removeInputHandler('ConvertDialog');
  invoke('convert_close').catch(() => {});
});
</script>
