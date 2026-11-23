<template>
  <div
    ref="container"
    class="relative isolate w-full h-full overflow-hidden cursor-default"
    style="touch-action: none;"
    @wheel="handleImageWheel"
  >

    <div
      v-if="infoTipLines.length > 0"
      class="absolute left-2 top-2 z-[60] max-w-[95%] flex flex-col items-start gap-1 pointer-events-none select-none"
    >
      <!-- separate tooltip above the info tooltip: load timings (diagnostics), only while enabled -->
      <div v-if="loadTimingText" class="max-w-full px-2 py-1 rounded bg-black/70 text-amber-300 text-xs font-mono whitespace-nowrap overflow-hidden text-ellipsis">{{ loadTimingText }}</div>
      <div class="max-w-full px-2 py-1 rounded bg-black/70 text-white text-xs font-mono">
        <div v-for="(line, i) in infoTipLines" :key="i" class="whitespace-nowrap overflow-hidden text-ellipsis">{{ line }}</div>
      </div>
    </div>

    <div
      v-if="loadTimingText && infoTipLines.length === 0"
      class="absolute left-2 top-2 z-[60] max-w-[95%] px-2 py-1 rounded bg-black/70 text-amber-300 text-xs font-mono whitespace-nowrap overflow-hidden text-ellipsis pointer-events-none select-none"
    >{{ loadTimingText }}</div>

    <!-- Loading overlay -->
    <transition name="fade">
      <div
        v-if="isLoading && !showInlineLoading && Number(fileType) !== 3"
        class="absolute inset-0 bg-base-100/50 flex items-center justify-center z-50 rounded-box"
      >
        <span class="loading loading-dots text-primary"></span>
      </div>
    </transition>

    <transition name="fade">
      <div
        v-if="isLoading && showInlineLoading && Number(fileType) !== 3"
        class="absolute left-2.5 bottom-2.5 z-50 pointer-events-none"
      >
        <span class="loading loading-spinner loading-xs text-primary/70"></span>
      </div>
    </transition>

    <div v-if="Number(fileType) === 3 && !offlinePreview" data-raw-preview-controls class="absolute left-4 bottom-4 z-60 flex items-center gap-2" @click.stop @dblclick.stop @mousedown.stop>
      <div class="inline-flex h-10 items-center gap-1 rounded-box bg-base-100/70 px-2 shadow" role="group" aria-label="RAW">
        <span class="px-1 text-sm font-medium">RAW</span>
        <button
          v-for="{ mode, icon } in rawPreviewButtons"
          :key="mode"
          type="button"
          class="inline-flex size-8 items-center justify-center rounded-box text-base-content/70 enabled:hover:bg-base-content/10 disabled:opacity-30 cursor-pointer"
          :class="{ 'bg-primary/15 text-primary': effectiveRawSource === mode }"
          :title="t(`settings.raw.source_${mode}`)"
          :aria-label="t(`settings.raw.source_${mode}`)"
          :aria-pressed="effectiveRawSource === mode"
          :disabled="rawModeDisabled(mode)"
          @click.stop="selectRawMode(mode)"
        >
          <span v-if="rawLoadingVisible && rawRequestPending && !rawModeDisabled(mode) && rawModeSource === mode && !requestedRawOptions.preferPair" class="loading loading-spinner loading-xs" aria-hidden="true"></span>
          <component v-else :is="icon" class="size-5" aria-hidden="true" />
        </button>
      </div>
      <button v-if="rawPairLabel && config.settings.groupRawJpegPairs" 
        class="inline-flex h-10 items-center rounded-box px-3 bg-base-100/70 hover:bg-base-100 text-sm font-medium shadow transition-colors cursor-pointer focus:outline-none"
        :class="effectiveRawSource === 'pair' ? 'text-primary hover:text-primary' : 'text-base-content/70 hover:text-base-content'" :aria-pressed="effectiveRawSource === 'pair'" :title="t('settings.raw.show_pair', { format: rawPairLabel })" @click.stop="selectRawPair">{{ rawPairLabel }}</button>
    </div>

    <!-- Error overlay -->
    <transition name="fade">
      <div v-if="loadError" class="absolute inset-0 bg-base-100/50 flex items-center justify-center z-50 rounded-box">
        <div class="h-full flex flex-col items-center justify-center text-base-content/30">
          <component :is="cloudLoadError ? IconCloud : IconError" class="w-8 h-8 mb-2" />
          <span>{{ $t(cloudLoadError ? 'offline.download_to_open' : offlinePreview ? 'tooltip.not_found.files' : 'image_viewer.failed') }}</span>
        </div>
      </div>
    </transition>

    <!-- main image -->
    <TransitionGroup :name="transitionName" @after-leave="handleTransitionEnd">
      <div 
        v-for="(src, index) in imageSrc"
        v-show="activeImage === index"
        :key="`img-${imageFilePath[index] || 'empty'}-${index}`"
        class="slide-wrapper absolute inset-0 w-full h-full pointer-events-none overflow-hidden"
      >
        <img
          ref="activeImageEl"
          :src="src"
          :class="isGrabbing ? (isDraggingImage ? 'cursor-grabbing' : 'cursor-grab') : 'cursor-default'"
          :style="getImageStyle(index)"
          draggable="false"
          @load="handleSlotLoad(index, $event)"
          @mousedown="handleImageMouseDown"
          @mousemove="handleImageMouseMove"
          @mouseup="handleImageMouseUp"
          @mouseleave="handleImageMouseLeave"
          @dblclick.stop="toggleZoomFit"
          @contextmenu.prevent.stop="$emit('context-menu', $event)"
        />
        <!-- Tiled mode (huge JPEG): the visible part decoded by the backend, over the preview. -->
        <div v-if="tileOverlay && index === activeImage && tileOverlay.path === imageFilePath[index]"
          :style="{ ...getImageStyle(index), pointerEvents: 'none', backgroundColor: 'transparent' }" aria-hidden="true">
          <img :src="tileOverlay.src" draggable="false" :style="tileOverlayImgStyle(index)" />
        </div>
        <!-- AF areas share the image buffer's layout and outer transition. -->
        <div v-if="showAfAreas && !loadError && index === activeImage && imageFilePath[index] === filePath"
          class="absolute inset-0 pointer-events-none overflow-hidden" aria-hidden="true">
          <div :style="{ ...getImageStyle(index), pointerEvents: 'none', filter: 'none', backgroundColor: 'transparent' }">
            <div v-for="(region, regionIndex) in afAreas" :key="regionIndex"
              class="absolute box-border"
              :style="{
                left: `${region.x * 100}%`, top: `${region.y * 100}%`,
                width: `${region.width * 100}%`, height: `${region.height * 100}%`,
                border: `${2 / scale[index]}px solid ${region.focused === true ? '#22c55e' : region.focused === false ? '#ef4444' : '#94a3b8'}`,
              }" />
          </div>
        </div>
      </div>
    </TransitionGroup>

    <!-- Faces Overlay -->
    <div 
      v-if="showFaceOverlay && faces.length > 0 && !isDraggingImage && !isSlideShow"
      class="absolute inset-0 w-full h-full pointer-events-none overflow-hidden"
    >
      <div
        class="absolute"
        :style="{
          width: `${imageSize[activeImage].width}px`,
          height: `${imageSize[activeImage].height}px`,
          transform: `translate(${position[activeImage].x}px, ${position[activeImage].y}px) 
                      scale(${scale[activeImage]}) 
                      rotate(${imageRotate[activeImage]}deg)`,
          transition: (!isDraggingImage || loupeAnimating) && !noTransition && !isResizingContainer ? (isDraggingNavBox ? 'transform 0.2s ease-out' : (loupeAnimating ? 'transform 0.15s ease-out' : 'transform 0.3s ease-in-out')) : 'none',
        }"
      >
        <div
          v-for="face in faces"
          :key="face.id"
          v-show="face.person_id === libConfig.person.id"
          class="absolute group"
          :style="{
            left: `${face.bbox.x}px`,
            top: `${face.bbox.y}px`,
            width: `${face.bbox.width}px`,
            height: `${face.bbox.height}px`,
          }"
        >
          <!-- Corner: Top Left -->
          <div 
            class="absolute top-0 left-0 w-1/8 h-1/8 border-secondary/50 rounded-tl-lg"
            :style="{ 
              borderTopWidth: `${2 / scale[activeImage]}px`, 
              borderLeftWidth: `${2 / scale[activeImage]}px` 
            }"
          ></div>
          
          <!-- Corner: Top Right -->
          <div 
            class="absolute top-0 right-0 w-1/8 h-1/8 border-secondary/50 rounded-tr-lg"
            :style="{ 
              borderTopWidth: `${2 / scale[activeImage]}px`, 
              borderRightWidth: `${2 / scale[activeImage]}px` 
            }"
          ></div>

          <!-- Corner: Bottom Left -->
          <div 
            class="absolute bottom-0 left-0 w-1/8 h-1/8 border-secondary/50 rounded-bl-lg"
            :style="{ 
              borderBottomWidth: `${2 / scale[activeImage]}px`, 
              borderLeftWidth: `${2 / scale[activeImage]}px` 
            }"
          ></div>

          <!-- Corner: Bottom Right -->
          <div 
            class="absolute bottom-0 right-0 w-1/8 h-1/8 border-secondary/50 rounded-br-lg"
            :style="{ 
              borderBottomWidth: `${2 / scale[activeImage]}px`, 
              borderRightWidth: `${2 / scale[activeImage]}px` 
            }"
          ></div>

          <!-- Person Name Tag (Hover) -->
          <!-- <div 
            class="hidden group-hover:flex absolute left-1/2 -translate-x-1/2 
                   bg-black/60 backdrop-blur-sm text-white text-xs font-medium 
                   px-2 py-1 rounded shadow-sm whitespace-nowrap z-10 pointer-events-none"
            :style="{
              bottom: '100%',
              marginBottom: `${8 / scale[activeImage]}px`,
              transform: `translateX(-50%) scale(${1 / scale[activeImage]})`,
              transformOrigin: 'bottom center',
            }"
          >
            {{ face.person_id ? `Person ${face.person_id}` : 'Unknown' }}
          </div> -->
        </div>
      </div>
    </div>

    <!-- Navigator view -->
    <transition name="fade">
      <div v-if="showNavigator" class="absolute right-4 bottom-4 z-20"
        :style="{ width: `${navContainerSize.width}px`, height: `${navContainerSize.height}px` }">
      <ImageNavigator
        class="h-full w-full outline outline-gray-50 shadow-lg shadow-gray-500"
        :style="{ width: `${navContainerSize.width}px`, height: `${navContainerSize.height}px` }"
        :source="displayThumbnailSrc || getThumbUrl(props.fileId, false, config.settings.thumbnailSize, props.fileVersion)"
        :viewport="navigatorViewport"
        :sharpness="config.settings.navigatorSharpnessGrid"
        @mouseenter="pauseNavigatorAutoHide"
        @mouseleave="resetNavigatorAutoHide"
        @navigate="navigateImage"
        @zoom="zoomNavigator"
        @toggle-fit="toggleZoomFit"
        @dragging="isDraggingNavBox = $event"
      />
      </div>
    </transition>

  </div>
</template>

<script setup lang="ts">
import { ref, shallowRef, toRaw, triggerRef, watch, onMounted, onBeforeUnmount, computed, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useUIStore } from '@/stores/uiStore';
import { config, libConfig } from '@/common/config';
import { zoomLocked } from '@/common/zoomLock';
import { followViewport } from '@/common/viewportFollow';
import { parseInfoTipFields, INFO_TIP_DEFAULT_LINE1, INFO_TIP_DEFAULT_LINE2, type InfoTipField } from '@/common/infoTip';
import { SIDEBAR, GESTURE_LOCK_THRESHOLD, HORIZONTAL_NAV_THRESHOLD, MOUSE_DRAG_NAV_THRESHOLD } from '@/common/constants';
import {
  getAssetSrc,
  getPreviewUrl,
  shouldUseBackendPreview,
  getFileExtension,
  getThumbUrl,
  getThumbnailDataUrl,
  getThumbnailDataUrlInflight,
  isWin,
  setThumbnailDataUrlInflight,
  formatTimestamp,
} from '@/common/utils';
import { getFacesForFile, getFileThumbById, getFfmpegBackedImageExtensions } from '@/common/api';
import { RawFace, Face } from '@/common/types';
import { rawDisplayKey, getRawDisplayOptions, appendRawDisplayParams, type RawPreviewSource, type RawDisplayOptions } from '@/common/rawDisplay';
import { useI18n } from 'vue-i18n';
import { useToast } from '@/common/toast';
import { createDragPreview, isWindowDragEdge, startNativeFileDrag } from '@/common/nativeDrag';
import { getSavedViewport, setSavedViewport, removeSavedViewport, type SavedViewport } from '@/common/savedViewports';

import { checkFileAccessibility, isCloudOnlyFile, setPngAlphaBackground } from '@/common/api';
import { setFileAccessibility } from '@/common/availability';
import { IconCloud, IconError, IconBrightness, IconEmbedded, IconRender } from '@/common/icons';
import ImageNavigator from '@/components/ImageNavigator.vue';

const { t } = useI18n();
const toast = useToast();
// Props
const props = defineProps({
  showAfAreas: { type: Boolean, default: false },
  afAreaFeedbackRequest: { type: Object, default: null },
  originalUnavailable: { type: Boolean, default: false },
  filePath: {
    type: String,
    required: false,
  },
  nextFilePath: {
    type: String,
    default: '',
  },
  // Paths of nearby images in preload priority order (next first, then previous).
  neighborFilePaths: {
    type: Array as () => Array<string | { path: string; id?: number; version?: number }>,
    default: () => [],
  },
  rotate: {
    type: Number,
    default: 0,
  },  
  isZoomFit: {
    type: Boolean,
    default: false,
  },
  isSlideShow: {
    type: Boolean,
    default: false,
  },
  slideShowTransitionMode: {
    type: Number,
    default: 0,
  },
  fileId: {
    type: Number,
    required: false,
  },
  fileType: {
    type: Number,
    default: 1,
  },
  rawPairPath: {
    type: String,
    default: '',
  },
  fileVersion: {
    type: Number,
    default: 0,
  },
  fileAlbumId: {
    type: Number,
    default: 0,
  },
  // Image Viewer window only: Save Viewport also carries the viewport to the following images.
  allowViewportFollow: {
    type: Boolean,
    default: false,
  },
  fileSize: {
    type: Number,
    default: 0,
  },
  fileCreatedAt: {
    type: Number,
    default: 0,
  },
  fileBitDepth: {
    type: Number,
    default: 0,
  },
  imageWidth: {
    type: Number,
    default: 0,
  },
  imageHeight: {
    type: Number,
    default: 0,
  },
  thumbnailSrc: {
    type: String,
    default: '',
  },
  showThumbnailPlaceholder: {
    type: Boolean,
    default: false,
  },
  suppressAutoNavigator: { type: Boolean, default: false },
  showInlineLoading: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(['message-from-image-viewer', 'scale', 'update:isZoomFit', 'viewport-change', 'context-menu', 'saved-viewport-change']);

type AfArea = { x: number; y: number; width: number; height: number; focused: boolean | null };
const afAreas = ref<AfArea[]>([]);
let afAreaRequest = 0;
let afAreaTimer: ReturnType<typeof setTimeout> | undefined;
let afAreaBusy = false;
let pendingAfArea: { request: number; filePath: string; notify: boolean } | null = null;
const loadAfAreas = async () => {
  if (afAreaBusy || !pendingAfArea) return;
  const target = pendingAfArea;
  pendingAfArea = null;
  afAreaBusy = true;
  try {
    const regions = await invoke<AfArea[]>('get_af_areas', { filePath: target.filePath });
    if (target.request === afAreaRequest) {
      afAreas.value = regions;
      if (target.notify && regions.length === 0) {
        toast.info(t('image_viewer.af_unavailable'), { placement: 'bottom-right' });
      }
    }
  } catch {
    // Unsupported or unreadable metadata must never interrupt image viewing.
  } finally {
    afAreaBusy = false;
    // Only the latest settled image may follow an in-flight request.
    if (pendingAfArea) void loadAfAreas();
  }
};
watch(() => [props.showAfAreas, props.filePath, props.fileVersion, props.originalUnavailable, props.afAreaFeedbackRequest], (_values, previousValues) => {
  const request = ++afAreaRequest;
  clearTimeout(afAreaTimer);
  pendingAfArea = null;
  afAreas.value = [];
  if (!props.showAfAreas || !props.filePath || props.originalUnavailable) return;
  const filePath = props.filePath;
  const feedback = props.afAreaFeedbackRequest;
  const notify = Boolean(previousValues && feedback && feedback !== previousValues[4]
    && feedback.filePath === filePath && feedback.fileVersion === props.fileVersion);
  // Holding an arrow key must not launch an unbounded number of RAW parses.
  afAreaTimer = setTimeout(() => {
    pendingAfArea = { request, filePath, notify };
    void loadAfAreas();
  }, 120);
}, { immediate: true });
onBeforeUnmount(() => {
  ++afAreaRequest;
  clearTimeout(afAreaTimer);
  pendingAfArea = null;
});

const uiStore = useUIStore();

// container
const container = ref(null);
const containerSize = ref({ width: 0, height: 0 });
const containerPos = ref({ x: 0, y: 0 });
const isZoomFit = ref(props.isZoomFit);     // Zoom to fit image in container

// image
const activeImage = ref(1);                 // which image is active (0 or 1)
const imageSrc = ref(['', '']);             // image source
const imageFilePath = ref(['', '']);        // source file path for each buffer image
const position = shallowRef([{ x: 0, y: 0 }, { x: 0, y: 0 }]); // Image position (top-left corner)
const scale = ref([1, 1]);                  // Image scale (zoom level)
const minScale = ref(0.1);                    // Minimum zoom level
const maxScale = ref(10);                   // Maximum zoom level
const getActualSizeScale = () => 1 / (window.devicePixelRatio || 1);
const getDisplayScale = (scaleValue: number) => scaleValue * (window.devicePixelRatio || 1);
const imageRotate = ref([0, 0]);            // Image rotation
const imageNaturalSize = ref([{ width: 0, height: 0 }, { width: 0, height: 0 }]);
const imageSize = ref([{ width: 0, height: 0 }, { width: 0, height: 0 }]);       // actual image size
const imageSizeRotated = ref([{ width: 0, height: 0 }, { width: 0, height: 0 }]); // image size after rotation

const isDraggingImage = ref(false);         // Dragging state

// Saved viewport (per-file zoom + position) state; see savedViewports.ts
const hasSavedViewport = ref(false);
let skipNextZoomFitWatch = false;
// Non-null while the displayed image comes from a saved viewport; holds the
// zoom-fit mode that was active before the restore.
let zoomFitBeforeRestore: boolean | null = null;

const isGrabbing = ref(false);              // Grabbing state
const navigatorAutoVisible = ref(true);
const showNavigator = computed(() =>
  config.settings.navigatorViewMode === 1
  || (config.settings.navigatorViewMode === 0 && !props.suppressAutoNavigator && isGrabbing.value && navigatorAutoVisible.value)
);
let navigatorAutoHideTimer: ReturnType<typeof setTimeout> | null = null;
const isResizingContainer = ref(false);
let resizeTransitionFrame = 0;
const noTransition = ref(false);            // Disable transition temporarily
const lastMousePosition = ref({ x: 0, y: 0 }); // Last mouse position for drag calculations
const mousePosition = ref({ x: 0, y: 0 });  // Current mouse position
const mouseDragNavDeltaX = ref(0);
const mouseDragNavDeltaY = ref(0);
const mouseDragNavTriggered = ref(false);

const faces = ref<any[]>([]); // Store faces for the current image
const showFaceOverlay = computed(() =>
  config.settings.face.enabled && config.main.sidebarIndex === SIDEBAR.PERSON
);

let animationFrameId: number | null = null;
const latestMouseEvent = ref<MouseEvent | null>(null);

// macOS touchpad wheel - accumulate delta values until they reach a threshold
let wheelDeltaAccumulator = 0;
let wheelThreshold = 10;
// Improved gesture handling for touchpad
const gestureType = ref<'none' | 'zoom' | 'nav'>('none');
let horizontalDeltaAccumulator = 0;
let verticalDeltaAccumulator = 0;
let gestureResetTimeout: NodeJS.Timeout | null = null;
let hasNavigatedThisGesture = false;
let lastDeltaX = 0;

const isWheelZooming = ref(false);
let wheelZoomTimeout: NodeJS.Timeout | null = null;

// Touchpad detection - sticky once detected
let isTouchpadDevice = false;

// Touchscreen gesture state
const activeTouchPointers = new Map<number, { x: number; y: number }>();
let pinchStartDistance = 0;
let pinchStartScale = 1;
let pinchCenter = { x: 0, y: 0 };
let isPinching = false;
// Single-finger pan
let panPointerId: number | null = null;
let panLastPos = { x: 0, y: 0 };
// Single-finger swipe navigation (only used when image fits / cannot pan)
let touchSwipeStart = { x: 0, y: 0 };
let touchSwipeTriggered = false;
// Suppresses synthesized mouse events while any touch is active so the
// existing mouse drag handler doesn't double-pan with our pointer logic.
const isTouchActive = ref(false);

// Swipe state
const navDirection = ref<'next' | 'prev' | ''>('');
const isSliding = computed(() => {
  if (props.isSlideShow) {
    return props.slideShowTransitionMode === 0;
  }
  return navDirection.value !== '';
});
const transitionName = computed(() => {
  if (props.isSlideShow) {
    if (props.slideShowTransitionMode === 1) return 'slideshow-fade';
    if (props.slideShowTransitionMode === 2) return '';
    return 'slide-next';
  }
  if (navDirection.value) {
    return navDirection.value === 'next' ? 'slide-next' : 'slide-prev';
  }
  return '';
});

// A slot shows the transparency colour only once its pixels are decoded; before
// that it shows the "unloaded photo" colour so the two are never confused.
const slotDecoded = ref<boolean[]>([false, false]);
function handleSlotLoad(index: number, event: Event) {
  const el = event.target as HTMLImageElement | null;
  if (!el) return;
  const key = imageFilePath.value[index];
  const done = () => {
    if (imageFilePath.value[index] === key) slotDecoded.value[index] = true;
  };
  el.decode().then(done, done);
}

// A will-change: transform layer keeps the highest raster scale it has had, so after a zoom to 100 %
// a huge photo at fit would be rasterised at full size (160 MP+ of tiles: black blocks, vanishing overlays).
const isHugeLayout = (index: number) => imageSize.value[index].width * imageSize.value[index].height >= 60_000_000;
const getImageStyle = (index: number) => ({
  position: 'absolute',
  // RAW placeholders and full previews can have different intrinsic sizes.
  // Use the same layout box as the centering, zoom and rotation calculations.
  width: `${imageSize.value[index].width}px`,
  height: `${imageSize.value[index].height}px`,
  maxWidth: 'none', // Override Tailwind's img max-width: 100%; zoom handles fitting.
  transform: `translate3d(${position.value[index].x}px, ${position.value[index].y}px, 0)
              scale(${scale.value[index]})
              rotate(${imageRotate.value[index]}deg)`,
  transition: !isSliding.value && (!isDraggingImage.value || loupeAnimating.value) && !noTransition.value && !isResizingContainer.value && !isWheelZooming.value
    ? (isDraggingNavBox.value ? 'transform 0.2s ease-out' : (loupeAnimating.value ? 'transform 0.15s ease-out' : 'transform 0.3s ease-in-out'))
    : 'none',
  willChange: isHugeLayout(index) ? 'auto' : 'transform',
  backfaceVisibility: isHugeLayout(index) ? 'visible' : 'hidden',
  pointerEvents: 'auto',
  // Visible only through transparent pixels (PNG, PSD, TIFF ...).
  backgroundColor: slotDecoded.value[index]
    ? (config.settings.transparencyColor || '#5a5a5a')
    : (config.settings.loadingBackgroundColor || '#000000'),
  filter: adjustmentStyle.value(index === activeImage.value ? imageSrc.value[activeImage.value] : imageSrc.value[index]),
});
// loading and error overlays
const isLoading = ref(false);
const loadError = ref(false);
const cloudLoadError = ref(false);
const offlinePreview = ref(false);
let loadingTimeout: NodeJS.Timeout | null = null;

// Show the RAW spinner only after loading has persisted for a moment, so fast
// loads don't flash it. Resets immediately when loading finishes.
const rawLoadingVisible = ref(false);
const currentLoadingId = ref(0);
let rawLoadingTimer: ReturnType<typeof setTimeout> | null = null;
watch([isLoading, currentLoadingId], ([loading]) => {
  if (rawLoadingTimer) { clearTimeout(rawLoadingTimer); rawLoadingTimer = null; }
  rawLoadingVisible.value = false;
  if (loading) {
    rawLoadingTimer = setTimeout(() => { rawLoadingVisible.value = true; rawLoadingTimer = null; }, 200);
  }
});

const activeImageEl = ref<HTMLImageElement | null>(null);
type LoadedImage = { src: string; naturalWidth: number; naturalHeight: number; layout?: { width: number; height: number }; img?: HTMLImageElement; perf?: { start: number; loaded: number; decoded: number }; raw?: { source: RawPreviewSource; unavailable: boolean; pair: string | null } };
const preloadCache = new Map<string, Promise<LoadedImage>>();
const rawOverride = ref<RawDisplayOptions | null>(null);
const rawSource = ref<RawPreviewSource>('');
const rawRequestPending = ref(false);
const rawSelectionVersion = ref(0);
const resolvedRawPairLabel = ref<string | null>(null);
const rawPairLabel = computed(() => resolvedRawPairLabel.value ?? (props.rawPairPath ? /\.(heic|heif|hif)$/i.test(props.rawPairPath) ? 'HEIC' : 'JPEG' : ''));
const rawEmbeddedUnavailable = ref(false);
const rawFullSize = ref(false);
let rawUpgradeFailed = false;
let rawUpgradeRetryAt = 0;
const rawViewerId = crypto.randomUUID();
let rawUpgradeTimer: ReturnType<typeof setTimeout> | null = null;
const requestedRawOptions = computed(() => rawOverride.value || getRawDisplayOptions());
// The RAW's own preview mode, independent of whether the paired JPEG/HEIC is
// the source currently on screen — the RAW button always shows this mode.
const rawModeSource = computed<'embedded' | 'rendered' | 'brightened'>(() => {
  const o = requestedRawOptions.value;
  const actual = rawSource.value;
  // While a mode switch is in flight, show the requested mode right away (the
  // spinner already signals loading) instead of waiting for the response.
  const switching = rawRequestPending.value && !!rawOverride.value;
  if (!switching && (actual === 'embedded' || actual === 'rendered' || actual === 'brightened')) {
    return actual;
  }
  if (o.mode === 'embedded' && !rawEmbeddedUnavailable.value) return 'embedded';
  return o.autoBright ? 'brightened' : 'rendered';
});
// Highlight the source confirmed by the preview response. A new file can
// fall back from embedded to rendered before its first preview is available.
const effectiveRawSource = rawSource;
const rawPreviewButtons = [
  { mode: 'embedded', icon: IconEmbedded },
  { mode: 'rendered', icon: IconRender },
  { mode: 'brightened', icon: IconBrightness },
] as const;
type RawMode = typeof rawPreviewButtons[number]['mode'];
const failedRawModes = ref(new Set<RawMode>());
function rawModeDisabled(mode: RawMode) {
  return failedRawModes.value.has(mode)
    || (mode === 'embedded' && (!rawSource.value || rawEmbeddedUnavailable.value));
}
function cycleRawPreview() {
  if (Number(props.fileType) !== 3 || props.originalUnavailable || offlinePreview.value || loadError.value) return false;
  if (rawRequestPending.value || isLoading.value) return true;

  const modes = rawPreviewButtons.filter(({ mode }) => !rawModeDisabled(mode));
  if (!modes.length) return true;
  const current = modes.findIndex(({ mode }) => mode === effectiveRawSource.value);
  const preferred = modes.find(({ mode }) => mode === rawModeSource.value);
  const next = current < 0 ? preferred ?? modes[0] : modes[(current + 1) % modes.length];
  selectRawMode(next.mode);
  return true;
}
function selectRawMode(mode: RawMode) {
  if (rawModeDisabled(mode) || (!rawRequestPending.value && effectiveRawSource.value === mode)) return;
  rawRequestPending.value = true;
  rawSelectionVersion.value++;
  rawOverride.value = { mode: mode === 'embedded' ? 'embedded' : 'rendered', autoBright: mode === 'embedded' ? getRawDisplayOptions().autoBright : mode === 'brightened', preferPair: false };
}
function selectRawPair() {
  if (rawSource.value === 'pair' && !rawRequestPending.value) return;
  rawRequestPending.value = true;
  rawSelectionVersion.value++;
  rawOverride.value = { ...requestedRawOptions.value, preferPair: true };
}
let rawAbortController: AbortController | null = null;
const rawObjectUrls = new Set<string>();
watch([() => props.filePath, () => props.fileVersion], () => {
  rawEmbeddedUnavailable.value = false;
  failedRawModes.value.clear();
}, { flush: 'sync' });
watch(() => props.filePath, () => {
  rawOverride.value = null;
  rawSource.value = '';
  resolvedRawPairLabel.value = null;
  rawRequestPending.value = false;
  rawFullSize.value = false;
}, { flush: 'sync' });
watch([rawDisplayKey, rawSelectionVersion, () => props.fileVersion, () => props.filePath], () => {
  rawFullSize.value = false;
  rawUpgradeFailed = false;
  rawUpgradeRetryAt = 0;
}, { flush: 'sync' });
watch(rawDisplayKey, () => { rawOverride.value = null; }, { flush: 'sync' });

async function loadRawImage(filePath: string, fullSize = false): Promise<LoadedImage> {
  rawAbortController?.abort();
  const controller = new AbortController();
  rawAbortController = controller;
  const url = new URL(getPreviewUrl(props.fileId, filePath, false, props.fileVersion));
  const options = requestedRawOptions.value;
  appendRawDisplayParams(url.searchParams, options);
  url.searchParams.set('rawViewerId', rawViewerId);
  if (fullSize) url.searchParams.set('rawFullSize', 'true');
  const response = await fetch(url.toString(), { signal: controller.signal });
  if (!response.ok) throw new Error('RAW preview failed');
  const blob = await response.blob();
  if (controller.signal.aborted) throw new Error('RAW preview cancelled');
  const src = URL.createObjectURL(blob);
  rawObjectUrls.add(src);
  try {
    const img = new Image();
    img.src = src;
    await img.decode();
    if (controller.signal.aborted) throw new Error('RAW preview cancelled');
    return { src, naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight, raw: {
      source: (response.headers.get('X-Raw-Source') || '') as RawPreviewSource,
      unavailable: response.headers.get('X-Raw-Embedded-Unavailable') === 'true',
      pair: options.preferPair ? response.headers.get('X-Raw-Pair') || '' : null,
    } };
  } catch (error) {
    URL.revokeObjectURL(src);
    rawObjectUrls.delete(src);
    throw error;
  }
}

watch(() => [scale.value[activeImage.value], rawSource.value, rawRequestPending.value, containerSize.value.width, containerSize.value.height], (current, previous) => {
  if (rawUpgradeTimer) clearTimeout(rawUpgradeTimer);
  if (rawUpgradeRetryAt && Date.now() >= rawUpgradeRetryAt
    && [0, 3, 4].some(index => current[index] !== previous[index])) {
    rawUpgradeFailed = false;
    rawUpgradeRetryAt = 0;
  }
  const index = activeImage.value;
  const natural = imageNaturalSize.value[index];
  const layout = imageSize.value[index];
  const pixelScale = scale.value[index] * (window.devicePixelRatio || 1);
  if (Number(props.fileType) !== 3 || !['rendered', 'brightened'].includes(rawSource.value)
    || rawFullSize.value || rawUpgradeFailed || rawRequestPending.value || isLoading.value
    || imageFilePath.value[index] !== props.filePath || props.originalUnavailable
    || (layout.width * pixelScale <= natural.width && layout.height * pixelScale <= natural.height)) return;
  const loadingId = currentLoadingId.value;
  const selectionVersion = rawSelectionVersion.value;
  const filePath = String(props.filePath);
  rawUpgradeTimer = setTimeout(async () => {
    rawUpgradeTimer = null;
    if (loadingId !== currentLoadingId.value || selectionVersion !== rawSelectionVersion.value || isLoading.value || rawRequestPending.value) return;
    rawRequestPending.value = true;
    try {
      const loaded = await loadRawImage(filePath, true);
      if (loadingId !== currentLoadingId.value || selectionVersion !== rawSelectionVersion.value) {
        URL.revokeObjectURL(loaded.src);
        rawObjectUrls.delete(loaded.src);
        return;
      }
      // A full-size render that fell back to the embedded preview must not
      // replace the sharper rendered preview or flip the user's selected mode.
      if (loaded.raw && loaded.raw.source !== 'rendered' && loaded.raw.source !== 'brightened') {
        URL.revokeObjectURL(loaded.src);
        rawObjectUrls.delete(loaded.src);
        rawUpgradeFailed = true;
        rawUpgradeRetryAt = Date.now() + 5000;
        return;
      }
      const slot = activeImage.value;
      const size = imageSize.value[slot];
      const previous = imageSrc.value[slot];
      const rotate = imageRotate.value[slot];
      // setImageSlot recomputes imageSizeRotated from props.rotate; preserve the
      // viewer's in-place rotation (and its rotated box) across the swap.
      const rotated = imageSizeRotated.value[slot];
      setImageSlot(slot, filePath, loaded.src, loaded.naturalWidth, loaded.naturalHeight, size.width, size.height);
      imageRotate.value[slot] = rotate;
      imageSizeRotated.value[slot] = rotated;
      if (loaded.raw) {
        rawSource.value = loaded.raw.source;
        rawEmbeddedUnavailable.value ||= loaded.raw.unavailable;
        rawFullSize.value = loaded.raw.source === 'rendered' || loaded.raw.source === 'brightened';
      }
      if (rawObjectUrls.delete(previous)) URL.revokeObjectURL(previous);
    } catch {
      if (loadingId === currentLoadingId.value && selectionVersion === rawSelectionVersion.value) {
        rawUpgradeFailed = true;
        rawUpgradeRetryAt = Date.now() + 5000;
        toast.warning(t('image_viewer.raw_render_unavailable'), { placement: 'bottom-right' });
      }
    } finally {
      if (loadingId === currentLoadingId.value && selectionVersion === rawSelectionVersion.value) rawRequestPending.value = false;
    }
  }, 150);
});

let resizeObserver: ResizeObserver | null = null;
const suppressViewportEmit = ref(false);
let warmImageTimeout: NodeJS.Timeout | null = null;
let warmImageIdleId: number | null = null;
const resolvedThumbnailSrc = ref('');
const resolvedThumbnailFileId = ref(0);
const displayThumbnailSrc = computed(() => props.thumbnailSrc || resolvedThumbnailSrc.value);

// Keep this list in Rust, alongside the preview implementation, so adding a
// new FFmpeg-backed format cannot reintroduce the placeholder/preview race.
let ffmpegBackedPreviewExtensions: Promise<Set<string>> | null = null;

function getFfmpegBackedPreviewExtensions() {
  if (!ffmpegBackedPreviewExtensions) {
    ffmpegBackedPreviewExtensions = getFfmpegBackedImageExtensions()
      .then((extensions: string[]) => new Set(extensions.map((extension) => extension.toLowerCase())));
  }
  return ffmpegBackedPreviewExtensions;
}

function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

// Big lossless PNGs are decoded by the backend (Rust) and served as a full-resolution JPEG,
// which WebView2 displays far faster than a 50-megapixel PNG.
const BACKEND_PNG_MIN_PIXELS = 12_000_000;
function usesBackendPreviewFor(filePath: string, fileType: number, meta?: { width?: number; height?: number }): boolean {
  if (shouldUseBackendPreview(filePath, fileType)) return true;
  if (config.settings.pngBackendPreview === false) return false;
  const backendExt = getFileExtension(filePath).toLowerCase();
  if (backendExt !== 'png' && backendExt !== 'bmp') return false;
  const sameAsCurrent = filePath === props.filePath;
  const w = Number(sameAsCurrent ? props.imageWidth : meta?.width) || 0;
  const h = Number(sameAsCurrent ? props.imageHeight : meta?.height) || 0;
  return w * h >= BACKEND_PNG_MIN_PIXELS;
}

// Backend previews that are already full resolution (big PNG, TIFF): no blurry thumbnail placeholder.
function isBackendPngPath(filePath: string): boolean {
  const ext = getFileExtension(filePath).toLowerCase();
  if (ext === 'tif' || ext === 'tiff') return true;
  return !shouldUseBackendPreview(filePath, Number(props.fileType || 0)) && usesBackendPreviewFor(filePath, Number(props.fileType || 0));
}

// The backend flattens transparent PNG pixels over the viewer's transparency colour.
function syncPngAlphaBackground() {
  const hex = String(config.settings.transparencyColor || '#5a5a5a').replace('#', '');
  const rgb = parseInt(hex.length === 3 ? hex.split('').map(c => c + c).join('') : hex, 16);
  if (Number.isFinite(rgb)) setPngAlphaBackground(rgb).catch(() => {});
}
watch(() => config.settings.transparencyColor, syncPngAlphaBackground, { immediate: true });

// inline loading for formats that require backend preview decoding
const showInlineLoading = computed(() =>
  props.showInlineLoading || (shouldUseBackendPreview(props.filePath, Number(props.fileType || 0)) && !!displayThumbnailSrc.value)
);

async function getEffectiveThumbnailSrc() {
  if (props.thumbnailSrc) return props.thumbnailSrc;
  const fileId = props.fileId;
  if (!fileId) return '';
  const thumbUrl = getThumbUrl(fileId, false, config.settings.thumbnailSize, props.fileVersion);
  if (!isWin) return thumbUrl;
  if (thumbUrl.startsWith('data:')) {
    resolvedThumbnailSrc.value = thumbUrl;
    resolvedThumbnailFileId.value = fileId;
    return thumbUrl;
  }
  if (resolvedThumbnailSrc.value && resolvedThumbnailFileId.value === fileId) return resolvedThumbnailSrc.value;

  const inflight = getThumbnailDataUrlInflight(fileId, config.settings.thumbnailSize);
  const dataUrl = await (inflight || setThumbnailDataUrlInflight(
    fileId,
    config.settings.thumbnailSize,
    getFileThumbById(fileId, config.settings.thumbnailSize, false)
      .then(thumb => getThumbnailDataUrl(thumb, '', false, config.settings.thumbnailSize, props.fileVersion))
  ));
  if (props.fileId === fileId && !props.thumbnailSrc && dataUrl) {
    resolvedThumbnailSrc.value = dataUrl;
    resolvedThumbnailFileId.value = fileId;
  }
  return props.fileId === fileId ? dataUrl || thumbUrl : '';
}

// navigator view mode
const navContainerSize = computed(() => {
  const max_size = config.settings.navigatorViewSize;
  const aspectRatio = imageSizeRotated.value[activeImage.value].width / imageSizeRotated.value[activeImage.value].height;
  if(aspectRatio >= 1) {
    return {
      width: max_size,
      height: Math.round(max_size / aspectRatio),
    };
  } else {
    return {
      width: Math.round(max_size * aspectRatio),
      height: max_size,
    };
  }
});

const adjustmentStyle = computed(() => (src: string) => {
  if (!uiStore.activeAdjustments.filePath) return '';
  if (uiStore.isInputActive('EditImage')) return '';
  
  // Check if current image matches the one being edited
  // We need to compare paths. The src might be a full URL (asset://...) while filePath is absolute path
  // But usually we can check if src contains the filePath or if we have the original filePath prop
  
  // Simpler: Check if the currently viewing file path matches the one in adjustment
  // defined in props.filePath or we can use the passed in src if we can decode it.
  
  // Actually, props.filePath is dependable for the *current* image, but we have two images in the DOM (previous and current).
  // The 'src' in the v-for loop is the asset URL.
  // Let's try to match loosely or use props.filePath if activeImage index matches
  
  // Better approach:
  // calculated adjustments should only apply if the source file matches the edited file.
  // However, mapping src (asset url) back to file path is tricky without util.
  // But we know 'src-vite' uses 'asset://' + filepath or similar.
  // Let's use a simpler heuristic: if props.filePath matches store's filePath, apply to the active image.
  
  if (uiStore.activeAdjustments.filePath === props.filePath) {
     const adj = uiStore.activeAdjustments;
     const parts = [];
     if (adj.brightness !== 0) parts.push(`brightness(${100 + adj.brightness}%)`);
     if (adj.contrast !== 0) parts.push(`contrast(${100 + adj.contrast}%)`);
     if (adj.saturation !== 100) parts.push(`saturate(${adj.saturation}%)`);
     if (adj.hue !== 0) parts.push(`hue-rotate(${adj.hue}deg)`);
     if (adj.blur > 0) parts.push(`blur(${adj.blur}px)`);
     if (adj.filter === 'grayscale') parts.push('grayscale(100%)');
     if (adj.filter === 'sepia') parts.push('sepia(100%)');
     if (adj.filter === 'invert') parts.push('invert(100%)');
     
     return parts.join(' ');
  }
  return '';
});

// ---- Hybrid mode (big JPEG / PNG) ----------------------------------------------
// A screen-sized preview made by the backend is shown at the photo's real layout size, so
// zoom and pan keep working in real pixels. The full-resolution file is swapped in only
// when the zoom needs more pixels than the preview has, and swapped out again at fit.
type HybridInfo = {
  path: string;
  meta?: { id?: number; version?: number; width?: number; height?: number; albumId?: number };
  layoutWidth: number;
  bitmapWidth: number;
  fitSrc: string;
  fullSrc: string | null;
  loadingFull: boolean;
  prefetchTimer: ReturnType<typeof setTimeout> | null;
};
const hybridSlot = ref<(HybridInfo | null)[]>([null, null]);
const fullCache = new Map<string, Promise<LoadedImage>>();
const HYBRID_MIN_PIXELS = 12_000_000;
const FULL_CACHE_LIMIT = 4;
// Photos wider or taller than this render broken at full resolution (missing tiles, even the
// overlays disappear; 19190 x 7971 is fine, 34092 x 7887 is not). They are not prefetched and not kept
// after zooming back to fit. The cache of full-resolution copies stays within a decoded-pixel budget.
const HUGE_FULL_EDGE = 20_000;
const FULL_CACHE_PIXEL_BUDGET = 500_000_000;
const fullPixels = new Map<string, number>();

function releaseFull(path: string) {
  fullCache.delete(path);
  fullPixels.delete(path);
}

function isHugeFull(info: { path: string; meta?: HybridInfo['meta'] }): boolean {
  // Photos shown tiled never load their original in the browser.
  if (usesTiles(info.path, info.meta)) return true;
  const { w, h } = hybridDims(info.path, info.meta);
  return Math.max(w, h) >= HUGE_FULL_EDGE;
}

// ---- Tiled mode (very big JPEG) ----------------------------------------------------
// The browser never decodes the original. When the zoom needs more pixels than the fit preview
// has, the backend decodes just the visible part (plus a margin) at the needed resolution and it is
// shown over the preview; it is replaced a moment after zooming or panning stops.
type TileOverlay = { path: string; src: string; x0: number; y0: number; x1: number; y1: number; outW: number };
const tileOverlay = ref<TileOverlay | null>(null);
let tileTimer: ReturnType<typeof setTimeout> | null = null;
let tileBusy = false;
let tileDirty = false;
let tileSeq = 0;
const TILE_MARGIN = 0.25;

function usesTiles(filePath: string, meta?: { width?: number; height?: number }): boolean {
  const mp = Number(config.settings.hybridTileMegapixels ?? 36);
  if (!(mp > 0)) return false;
  const ext = getFileExtension(filePath).toLowerCase();
  if (!['jpg', 'jpeg', 'jpe', 'tif', 'tiff', 'psd', 'psb'].includes(ext)) return false;
  const { w, h } = hybridDims(filePath, meta);
  return w * h >= Math.max(mp, 12) * 1_000_000;
}

function tileOverlayImgStyle(index: number) {
  const o = tileOverlay.value;
  const size = imageSize.value[index];
  if (!o || !(size.width > 0) || !(size.height > 0)) return {};
  // The tile holds ~1 pixel per screen pixel, but its box sits at a fractional screen position, so the
  // browser's smoothing would blur it by up to half a pixel. At ~1:1 use nearest-neighbour instead: it
  // just snaps to the pixel grid and stays crisp. Far from 1:1 keep normal smoothing (no aliasing).
  const dpr = window.devicePixelRatio || 1;
  const density = o.outW / Math.max(1, o.x1 - o.x0);   // tile pixels per source pixel
  const wanted = (scale.value[index] || 1) * dpr;     // screen pixels per source pixel
  const crisp = wanted > 0 && Math.abs(density / wanted - 1) < 0.02;
  return {
    position: 'absolute',
    maxWidth: 'none',
    imageRendering: crisp ? 'pixelated' : 'auto',
    left: `${(o.x0 / size.width) * 100}%`,
    top: `${(o.y0 / size.height) * 100}%`,
    width: `${((o.x1 - o.x0) / size.width) * 100}%`,
    height: `${((o.y1 - o.y0) / size.height) * 100}%`,
  };
}

// Region (layout = source pixels) the viewer needs now, or 'hide' when the preview is sharp enough.
function computeTileRequest(slot: number): 'hide' | { x0: number; y0: number; x1: number; y1: number; outW: number; vx0: number; vy0: number; vx1: number; vy1: number } {
  const info = hybridSlot.value[slot];
  const size = imageSize.value[slot];
  const cont = containerSize.value;
  const s = scale.value[slot];
  const p = position.value[slot];
  if (!info || !(size.width > 0) || !(size.height > 0) || !(cont.width > 0) || !(s > 0)) return 'hide';
  if ((imageRotate.value[slot] || 0) % 360 !== 0) return 'hide';
  const dpr = window.devicePixelRatio || 1;
  if (s * dpr * size.width <= info.bitmapWidth * 1.02) return 'hide';
  // Layout x of container x: u = (X - p.x - W / 2) / s + W / 2 (the box is scaled about its centre).
  const toU = (X: number) => (X - p.x - size.width / 2) / s + size.width / 2;
  const toV = (Y: number) => (Y - p.y - size.height / 2) / s + size.height / 2;
  const clampX = (v: number) => Math.min(size.width, Math.max(0, v));
  const clampY = (v: number) => Math.min(size.height, Math.max(0, v));
  const vx0 = clampX(toU(0));
  const vx1 = clampX(toU(cont.width));
  const vy0 = clampY(toV(0));
  const vy1 = clampY(toV(cont.height));
  if (!(vx1 > vx0) || !(vy1 > vy0)) return 'hide';
  const mx = cont.width * TILE_MARGIN / s;
  const my = cont.height * TILE_MARGIN / s;
  const x0 = Math.floor(clampX(toU(0) - mx));
  const x1 = Math.ceil(clampX(toU(cont.width) + mx));
  const y0 = Math.floor(clampY(toV(0) - my));
  const y1 = Math.ceil(clampY(toV(cont.height) + my));
  if (x1 - x0 < 1 || y1 - y0 < 1) return 'hide';
  const outW = Math.max(1, Math.min(x1 - x0, Math.round((x1 - x0) * s * dpr)));
  return { x0, y0, x1, y1, outW, vx0, vy0, vx1, vy1 };
}

function tileCovers(req: Exclude<ReturnType<typeof computeTileRequest>, 'hide'>): boolean {
  const o = tileOverlay.value;
  if (!o) return false;
  if (req.vx0 < o.x0 || req.vy0 < o.y0 || req.vx1 > o.x1 || req.vy1 > o.y1) return false;
  const have = o.outW / (o.x1 - o.x0);
  const need = req.outW / (req.x1 - req.x0);
  return have >= need * 0.95;
}

function scheduleTileOverlay() {
  if (tileTimer) clearTimeout(tileTimer);
  tileTimer = setTimeout(() => { tileTimer = null; void runTileOverlay(); }, 120);
}

function onTileViewChange() {
  const slot = activeImage.value;
  const info = hybridSlot.value[slot];
  if (tileOverlay.value && tileOverlay.value.path !== imageFilePath.value[slot]) tileOverlay.value = null;
  if (tilePre && tilePre.path !== imageFilePath.value[slot]) { tilePre = null; tilePreImg = null; }
  if (!info || imageFilePath.value[slot] !== info.path || !usesTiles(info.path, info.meta)) return;
  if (computeTileRequest(slot) === 'hide') {
    tileOverlay.value = null;
    if (tileTimer) { clearTimeout(tileTimer); tileTimer = null; }
    // At rest in the fit view: prepare the 100 % region around the centre.
    if (tilePreTimer) clearTimeout(tilePreTimer);
    tilePreTimer = setTimeout(() => {
      tilePreTimer = null;
      if (config.settings.hybridTileCenter === true) {
        const size = imageSize.value[activeImage.value];
        prefetchTileAround(size.width / 2, size.height / 2);
      } else if (tileLastCursor) {
        // Not the centre: the region under the cursor (if it rests on the photo).
        prefetchTileAtCursor(tileLastCursor);
      }
    }, 400);
    return;
  }
  if (useTilePrefetched()) return;
  scheduleTileOverlay();
}

// ---- Tiled mode: regions prepared in advance ------------------------------------
// One region of twice the window size at 100 % (1 source pixel per device pixel) is kept ready, made
// around the photo centre at rest and around the cursor when the button goes down. It serves any zoom
// whose visible part lies inside it, so the first zoom shows sharp pixels at once.
let tilePre: TileOverlay | null = null;
let tilePreImg: HTMLImageElement | null = null; // keeps the decoded bitmap alive
let tilePrePromise: Promise<void> | null = null;
let tilePreBusyKey = '';
let tilePreTimer: ReturnType<typeof setTimeout> | null = null;

function tileRegionUrl(info: HybridInfo, x0: number, y0: number, x1: number, y1: number, outW: number): string {
  const base = getPreviewUrl(info.meta?.id ?? props.fileId, info.path, false, info.meta?.version ?? props.fileVersion);
  if (!base) return '';
  const sharp = Math.min(100, Math.max(0, Number(config.settings.hybridSharpness ?? 40)));
  const filter = Math.min(2, Math.max(0, Number(config.settings.hybridFilter ?? 0)));
  return withQuery(base, `region=${x0},${y0},${x1},${y1}&out=${outW}&sharp=${sharp}&filter=${filter}${config.settings.hybridTileQuality === false ? '' : '&hq=1'}`);
}

function prefetchTileAround(u: number, v: number) {
  const slot = activeImage.value;
  const info = hybridSlot.value[slot];
  if (config.settings.hybridTilePrefetch === false || !info || imageFilePath.value[slot] !== info.path || !usesTiles(info.path, info.meta)) return;
  if ((imageRotate.value[slot] || 0) % 360 !== 0) return;
  const size = imageSize.value[slot];
  const cont = containerSize.value;
  if (!(size.width > 0) || !(size.height > 0) || !(cont.width > 0)) return;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.min(Math.round(size.width), Math.round(2 * cont.width * dpr));
  const h = Math.min(Math.round(size.height), Math.round(2 * cont.height * dpr));
  // What a 100 % view around this point needs.
  const needW = cont.width * dpr;
  const needH = cont.height * dpr;
  const nx0 = Math.max(0, u - needW / 2);
  const nx1 = Math.min(size.width, u + needW / 2);
  const ny0 = Math.max(0, v - needH / 2);
  const ny1 = Math.min(size.height, v + needH / 2);
  if (tilePre && tilePre.path === info.path && nx0 >= tilePre.x0 && nx1 <= tilePre.x1 && ny0 >= tilePre.y0 && ny1 <= tilePre.y1) return;
  const x0 = Math.round(Math.min(Math.max(0, u - w / 2), size.width - w));
  const y0 = Math.round(Math.min(Math.max(0, v - h / 2), size.height - h));
  const key = `${info.path}|${x0},${y0},${w},${h}`;
  if (tilePreBusyKey === key) return;
  const src = tileRegionUrl(info, x0, y0, x0 + w, y0 + h, w);
  if (!src) return;
  tilePreBusyKey = key;
  const startedAt = performance.now();
  const img = new Image();
  img.decoding = 'async';
  img.src = src;
  const promise: Promise<void> = img.decode().then(() => {
    if (imageFilePath.value[activeImage.value] !== info.path) return;
    noteTileTiming('ahead', startedAt, w, h);
    tilePre = { path: info.path, src, x0, y0, x1: x0 + w, y1: y0 + h, outW: w };
    tilePreImg = img;
  }).catch(() => {}).finally(() => {
    if (tilePreBusyKey === key) tilePreBusyKey = '';
    if (tilePrePromise === promise) tilePrePromise = null;
  });
  tilePrePromise = promise;
}

// Shows the prepared region when it holds what the view needs; true when it is (now) on screen.
function useTilePrefetched(): boolean {
  const slot = activeImage.value;
  const o = tilePre;
  if (!o || o.path !== imageFilePath.value[slot]) return false;
  const req = computeTileRequest(slot);
  if (req === 'hide') return false;
  if (tileOverlay.value?.src === o.src) return true;
  if (tileCovers(req)) return false; // the overlay on screen already does the job
  if (req.vx0 < o.x0 || req.vy0 < o.y0 || req.vx1 > o.x1 || req.vy1 > o.y1) return false;
  if (o.outW / (o.x1 - o.x0) < (req.outW / (req.x1 - req.x0)) * 0.95) return false;
  tileOverlay.value = o;
  return true;
}

// Cursor resting on the preview of a tiled photo: prepare the 100 % region under it.
let tileHoverTimer: ReturnType<typeof setTimeout> | null = null;
let tileHoverArmed = false;
let tileLastCursor: { clientX: number; clientY: number } | null = null;
let tileHoverStart = { x: 0, y: 0 };
function clearTileHover() {
  if (tileHoverTimer) { clearTimeout(tileHoverTimer); tileHoverTimer = null; }
  tileHoverArmed = false;
  tileLastCursor = null;
}
function onTileHover(event: MouseEvent) {
  tileLastCursor = { clientX: event.clientX, clientY: event.clientY };
  const ms = Number(config.settings.hybridTileHoverMs ?? 1000);
  if (!(ms > 0) || config.settings.hybridTilePrefetch === false || isDraggingImage.value) { clearTileHover(); return; }
  tileLastCursor = { clientX: event.clientX, clientY: event.clientY };
  const radius = Math.max(1, Number(config.settings.hybridTileHoverRadius ?? 20));
  // Still inside the radius of the point where the wait started: keep waiting (or stay done).
  if (tileHoverArmed && Math.hypot(event.clientX - tileHoverStart.x, event.clientY - tileHoverStart.y) <= radius) return;
  clearTileHover();
  const slot = activeImage.value;
  const info = hybridSlot.value[slot];
  if (!info || imageFilePath.value[slot] !== info.path || !usesTiles(info.path, info.meta)) return;
  if (computeTileRequest(slot) !== 'hide') return; // only while the preview is shown
  tileHoverArmed = true;
  tileHoverStart = { x: event.clientX, y: event.clientY };
  const point = { clientX: event.clientX, clientY: event.clientY };
  tileHoverTimer = setTimeout(() => {
    tileHoverTimer = null;
    if (computeTileRequest(activeImage.value) === 'hide') prefetchTileAtCursor(point);
  }, ms);
}

// Duration of the last tile region, shown with the load timings.
function noteTileTiming(kind: string, startedAt: number, w: number, h: number) {
  if (!config.settings.showLoadTiming) return;
  const part = `TILE ${kind} ${Math.round(performance.now() - startedAt)} ms ${w}x${h}`;
  const base = loadTimingText.value.replace(/ ?\| TILE .*$|^TILE .*$/, '');
  loadTimingText.value = base ? `${base} | ${part}` : part;
}

// Button pressed on the photo: a zoom to 100 % around the cursor may follow.
function prefetchTileAtCursor(event: { clientX: number; clientY: number }) {
  const slot = activeImage.value;
  const size = imageSize.value[slot];
  const s = scale.value[slot];
  const p = position.value[slot];
  if (!(s > 0) || !(size.width > 0)) return;
  const X = event.clientX - containerPos.value.x;
  const Y = event.clientY - containerPos.value.y;
  prefetchTileAround((X - p.x - size.width / 2) / s + size.width / 2, (Y - p.y - size.height / 2) / s + size.height / 2);
}

async function runTileOverlay() {
  const slot = activeImage.value;
  const info = hybridSlot.value[slot];
  if (!info || imageFilePath.value[slot] !== info.path || !usesTiles(info.path, info.meta)) return;
  const req = computeTileRequest(slot);
  if (req === 'hide') { tileOverlay.value = null; return; }
  if (tileCovers(req)) return;
  // A region being prepared may be just what is needed: wait for it instead of decoding twice.
  if (tilePrePromise) {
    await tilePrePromise;
    if (useTilePrefetched()) return;
    if (computeTileRequest(slot) === 'hide') return;
  }
  if (tileBusy) { tileDirty = true; return; }
  const base = getPreviewUrl(info.meta?.id ?? props.fileId, info.path, false, info.meta?.version ?? props.fileVersion);
  if (!base) return;
  tileBusy = true;
  const seq = ++tileSeq;
  try {
    const sharp = Math.min(100, Math.max(0, Number(config.settings.hybridSharpness ?? 40)));
    const filter = Math.min(2, Math.max(0, Number(config.settings.hybridFilter ?? 0)));
    const src = withQuery(base, `region=${req.x0},${req.y0},${req.x1},${req.y1}&out=${req.outW}&sharp=${sharp}&filter=${filter}${config.settings.hybridTileQuality === false ? '' : '&hq=1'}`);
    const startedAt = performance.now();
    const img = new Image();
    img.decoding = 'async';
    img.src = src;
    await img.decode();
    noteTileTiming('view', startedAt, req.outW, Math.round(req.outW * (req.y1 - req.y0) / (req.x1 - req.x0)));
    if (seq === tileSeq && imageFilePath.value[activeImage.value] === info.path) {
      tileOverlay.value = { path: info.path, src, x0: req.x0, y0: req.y0, x1: req.x1, y1: req.y1, outW: req.outW };
    }
  } catch {
    // The region could not be made: the preview stays.
    if (config.settings.showLoadTiming) loadTimingText.value = `TILE failed: region ${req.x0},${req.y0}-${req.x1},${req.y1} out ${req.outW}px`;
  } finally {
    tileBusy = false;
    // Always look again: the view may have moved while the region was decoded.
    tileDirty = false;
    scheduleTileOverlay();
  }
}

function hybridDims(filePath: string, meta?: { width?: number; height?: number }): { w: number; h: number } {
  const sameAsCurrent = filePath === props.filePath;
  return {
    w: Number(sameAsCurrent ? props.imageWidth : meta?.width) || 0,
    h: Number(sameAsCurrent ? props.imageHeight : meta?.height) || 0,
  };
}

function isHybridCandidate(filePath: string, meta?: { id?: number; width?: number; height?: number }): boolean {
  if (config.settings.hybridPreview === false) return false;
  if (!((filePath === props.filePath ? props.fileId : meta?.id) > 0)) return false;
  if (Number(props.fileType || 0) === 3) return false;
  const ext = getFileExtension(filePath).toLowerCase();
  if (!['jpg', 'jpeg', 'jpe', 'png', 'bmp', 'tif', 'tiff', 'psd', 'psb'].includes(ext)) return false;
  const { w, h } = hybridDims(filePath, meta);
  return w * h >= HYBRID_MIN_PIXELS;
}

// Longest side (device pixels) a fit preview needs: the largest screen dimension.
function hybridFitEdge(): number {
  const dpr = window.devicePixelRatio || 1;
  const longest = Math.max(window.screen?.width || 1920, window.screen?.height || 1080) * dpr;
  return Math.min(8192, Math.max(2048, Math.ceil(longest / 256) * 256));
}

// Should the backend keep this photo's fit preview on disk? Optionally not for photos browsed
// through the Disk tab (browse-only albums), only for photos in real albums.
function hybridDiskCacheFor(filePath: string, meta?: { albumId?: number }): boolean {
  if (config.settings.hybridDiskCache === false) return false;
  if (config.settings.hybridDiskCacheAlbumsOnly === false) return true;
  const albumId = Number(filePath === props.filePath ? props.fileAlbumId : meta?.albumId) || 0;
  if (albumId <= 0) return true;
  const diskAlbums = ((config.main as any).diskAlbumIds || []).map(Number);
  return !diskAlbums.includes(albumId);
}

function withQuery(url: string, query: string): string {
  return `${url}${url.includes('?') ? '&' : '?'}${query}`;
}

// True layout size of a photo whose preview bitmap is bw x bh; the database size may be
// stored before EXIF orientation, so a swapped aspect is accepted.
function resolveHybridLayout(bw: number, bh: number, dbW: number, dbH: number): { width: number; height: number } | null {
  if (!(bw > 0) || !(bh > 0) || !(dbW > 0) || !(dbH > 0)) return null;
  let w = dbW;
  let h = dbH;
  if ((bw >= bh) !== (w >= h) && Math.abs(bw / bh - 1) > 0.02) [w, h] = [h, w];
  if (Math.abs(bw / bh - w / h) / (w / h) > 0.02) return null;
  return { width: w, height: h };
}

const hybridLayouts = new Map<string, { path: string; layout: { width: number; height: number }; meta?: HybridInfo['meta'] }>();

function markHybridSlot(slot: number, filePath: string, src: string, bitmapWidth: number) {
  const previous = hybridSlot.value[slot];
  if (previous?.prefetchTimer) clearTimeout(previous.prefetchTimer);
  const entry = hybridLayouts.get(src);
  if (!entry || entry.path !== filePath) {
    // Not a hybrid fit preview (could be the full-resolution swap itself).
    if (!(previous && previous.path === filePath && previous.fullSrc === src)) hybridSlot.value[slot] = null;
    return;
  }
  const info: HybridInfo = {
    path: filePath,
    meta: entry.meta,
    layoutWidth: entry.layout.width,
    bitmapWidth,
    fitSrc: src,
    fullSrc: null,
    loadingFull: false,
    prefetchTimer: null,
  };
  hybridSlot.value[slot] = info;
  // A moment after the photo is shown, prepare the full resolution so zooming in is instant.
  info.prefetchTimer = setTimeout(() => {
    info.prefetchTimer = null;
    // hybridSlot is a ref: reading it returns a reactive proxy, so compare the raw objects.
    if (toRaw(hybridSlot.value[slot]) === info && !isHugeFull(info)) void loadHybridFull(info);
  }, 0);
}

// Called when a zoom to a high magnification is likely (press on the photo): start loading the
// full resolution right away so the swap is as early as possible.
function prefetchActiveHybridFull() {
  const info = hybridSlot.value[activeImage.value];
  if (info && imageFilePath.value[activeImage.value] === info.path && !isHugeFull(info)) void loadHybridFull(info);
}

function loadHybridFull(info: HybridInfo): Promise<LoadedImage> {
  let promise = fullCache.get(info.path);
  if (!promise) {
    promise = loadImageResource(info.path, info.meta, { full: true });
    fullCache.set(info.path, promise);
    promise.then((loaded) => {
      if (fullCache.get(info.path) !== promise) return;
      fullPixels.set(info.path, loaded.naturalWidth * loaded.naturalHeight);
      let total = 0;
      for (const px of fullPixels.values()) total += px;
      // Evict the oldest entries (never the one just loaded) while over the count or pixel budget.
      for (const key of Array.from(fullCache.keys())) {
        if (key === info.path) continue;
        if (fullCache.size <= FULL_CACHE_LIMIT && total <= FULL_CACHE_PIXEL_BUDGET) break;
        total -= fullPixels.get(key) || 0;
        releaseFull(key);
      }
    }).catch(() => releaseFull(info.path));
    while (fullCache.size > FULL_CACHE_LIMIT) {
      const oldest = fullCache.keys().next().value;
      if (oldest === undefined) break;
      releaseFull(oldest);
    }
  }
  return promise;
}

function updateHybridResolution() {
  const slot = activeImage.value;
  const info = hybridSlot.value[slot];
  if (!info || imageFilePath.value[slot] !== info.path) return;
  const dpr = window.devicePixelRatio || 1;
  const displayed = scale.value[slot] * dpr * info.layoutWidth;
  if (usesTiles(info.path, info.meta)) return; // tiled mode: handled by onTileViewChange
  const showingFull = info.fullSrc !== null && imageSrc.value[slot] === info.fullSrc;
  if (!showingFull && displayed > info.bitmapWidth * 1.05) {
    if (info.loadingFull) return;
    info.loadingFull = true;
    const tNeed = performance.now();
    const alreadyRequested = fullCache.has(info.path);
    loadHybridFull(info).then((full) => {
      info.loadingFull = false;
      if (config.settings.showLoadTiming) {
        const tReady = performance.now();
        const p = full.perf;
        const backend = p ? `fetch ${Math.round(p.loaded - p.start)} ms, decode ${Math.round(p.decoded - p.loaded)} ms` : 'n/a';
        const waited = Math.round(tReady - tNeed);
        const ago = p ? Math.round(tNeed - p.start) : 0;
        requestAnimationFrame(() => requestAnimationFrame(() => {
          const painted = Math.round(performance.now() - tNeed);
          loadTimingText.value = `FULL: ${alreadyRequested ? 'prefetched' : 'requested late'} ${ago} ms before need | ${backend} | waited ${waited} ms | painted +${painted} ms | ${full.naturalWidth}x${full.naturalHeight}`;
          console.info('[lap-full]', info.path, loadTimingText.value);
        }));
      }
      if (hybridSlot.value[slot] !== info || imageFilePath.value[slot] !== info.path) return;
      info.fullSrc = full.src;
      const stillNeeded = scale.value[slot] * dpr * info.layoutWidth > info.bitmapWidth * 1.05;
      if (stillNeeded) {
        imageNaturalSize.value[slot] = { width: full.naturalWidth, height: full.naturalHeight };
        imageSrc.value[slot] = full.src;
      }
    }).catch(() => { info.loadingFull = false; });
  } else if (showingFull && displayed <= info.bitmapWidth * 1.02) {
    // Back at fit: the fit preview is sharper and consistent than a GPU-downscaled original.
    // The preview is about as wide as the screen, so at fit the displayed width is ~100 % of the
    // bitmap width (it used to need < 90 %, which a photo fitted to the screen never reaches).
    // Swapping forward needs > 105 %, so there is no flapping in between.
    imageNaturalSize.value[slot] = { width: info.bitmapWidth, height: Math.round(info.bitmapWidth * (imageSize.value[slot].height / imageSize.value[slot].width)) };
    imageSrc.value[slot] = info.fitSrc;
    // A huge original is dropped at fit (its decoded copy would stay in memory); zooming in loads it again.
    if (isHugeFull(info)) {
      releaseFull(info.path);
      info.fullSrc = null;
    }
  }
}
watch(() => [activeImage.value, scale.value[activeImage.value]], updateHybridResolution);
watch(() => [
  activeImage.value,
  imageFilePath.value[activeImage.value],
  scale.value[activeImage.value],
  position.value[activeImage.value]?.x,
  position.value[activeImage.value]?.y,
  imageRotate.value[activeImage.value],
  containerSize.value.width,
  containerSize.value.height,
  hybridSlot.value[activeImage.value]?.path,
], onTileViewChange);

// ---- Info tooltip (top left): file name and details ------------------------------
function formatInfoTipSize(bytes: number): string {
  if (!(bytes > 0)) return '';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / Math.pow(1024, i);
  return `${i === 0 ? value.toFixed(0) : value.toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} ${units[i]}`;
}

function buildInfoTipLine(fields: InfoTipField[]): string {
  const segments: string[] = [];
  const index = activeImage.value;
  for (const field of fields) {
    switch (field) {
      case 'name': {
        const path = String(props.filePath || '');
        const name = path.split(/[\\/]/).pop() || '';
        if (name) segments.push(name);
        break;
      }
      case 'path':
        if (props.filePath) segments.push(String(props.filePath));
        break;
      case 'dimensions': {
        const size = imageSizeRotated.value[index];
        const w = Math.round(size?.width || props.imageWidth || 0);
        const h = Math.round(size?.height || props.imageHeight || 0);
        if (w > 0 && h > 0) segments.push(`${w} x ${h}`);
        break;
      }
      case 'bitDepth':
        if (props.fileBitDepth > 0) {
          const text = `${props.fileBitDepth}b`;
          // Next to the dimensions it reads "10378 x 4900 x 8b".
          if (fields.includes('dimensions') && segments.length > 0 && /^\d+ x \d+$/.test(segments[segments.length - 1])) {
            segments[segments.length - 1] += ` x ${text}`;
          } else {
            segments.push(text);
          }
        }
        break;
      case 'size': {
        const text = formatInfoTipSize(props.fileSize);
        if (text) segments.push(text);
        break;
      }
      case 'created': {
        const text = props.fileCreatedAt > 0 ? formatTimestamp(props.fileCreatedAt, 'date_time') : '';
        if (text) segments.push(text);
        break;
      }
      case 'zoom':
        if (scale.value[index] > 0) segments.push(`${Math.round(scale.value[index] * (window.devicePixelRatio || 1) * 100)}%`);
        break;
      case 'flags':
        if (zoomLocked.value) segments.push('LOCK');
        if (hasSavedViewport.value) segments.push('VIEWPORT');
        // Hybrid mode: the screen-sized preview is shown (full resolution not swapped in yet).
        if (tileOverlay.value && tileOverlay.value.path === imageFilePath.value[index]) segments.push('TILES');
        else if (imageNaturalSize.value[index].width > 0 && imageNaturalSize.value[index].width < imageSize.value[index].width * 0.98) segments.push('PREVIEW');
        break;
    }
  }
  return segments.join(' | ');
}

const infoTipLines = computed<string[]>(() => {
  if (config.settings.infoTipEnabled === false || props.isSlideShow) return [];
  if (!props.filePath || loadError.value) return [];
  const line1 = buildInfoTipLine(parseInfoTipFields(config.settings.infoTipLine1, INFO_TIP_DEFAULT_LINE1));
  const line2 = buildInfoTipLine(parseInfoTipFields(config.settings.infoTipLine2, INFO_TIP_DEFAULT_LINE2));
  const line3 = buildInfoTipLine(parseInfoTipFields(config.settings.infoTipLine3, ''));
  return [line1, line2, line3].filter((line) => line.length > 0);
});

// ---- Load timing (diagnostics, enabled by settings.showLoadTiming) ----------
const loadTimingText = ref('');
watch(() => config.settings.showLoadTiming, (on) => { if (!on) loadTimingText.value = ''; });
function reportLoadTiming(path: string, t0: number, tCloud: number, loaded: LoadedImage, tSlot: number, tPaint: number, fromCache: boolean, stageMs = 0) {
  if (!config.settings.showLoadTiming) { loadTimingText.value = ''; return; }
  const r = (n: number) => Math.round(n);
  const p = loaded.perf;
  const parts = [
    `cloud-check ${r(tCloud - t0)} ms`,
    p && !fromCache ? `fetch+header ${r(p.loaded - p.start)} ms` : 'preloaded (cache)',
    p && !fromCache ? `decode ${r(p.decoded - p.loaded)} ms` : '',
    stageMs > 0 ? `stage ${r(stageMs)} ms` : '',
    `show ${r(tPaint - tSlot)} ms`,
    `TOTAL ${r(tPaint - t0)} ms`,
    loaded.layout ? `${loaded.layout.width}x${loaded.layout.height} (fit ${loaded.naturalWidth}px)` : `${loaded.naturalWidth}x${loaded.naturalHeight}`,
  ].filter(Boolean);
  loadTimingText.value = parts.join(' | ');
  console.info('[lap-load]', path, loadTimingText.value);
}

function loadImageResource(filePath?: string, meta?: { id?: number; version?: number; width?: number; height?: number; albumId?: number }, opts: { full?: boolean } = {}) {
  if (!filePath) {
    return Promise.reject(new Error('Missing file path'));
  }

  if (Number(props.fileType) === 3) return loadRawImage(filePath);

  const full = !!opts.full;
  const hybrid = !full && isHybridCandidate(filePath, meta);
  const cached = full ? undefined : preloadCache.get(filePath);
  if (cached) {
    return cached;
  }

  const loadPromise = new Promise<LoadedImage>((resolve, reject) => {
    let src = '';
    const perfStart = performance.now();
    let perfLoaded = 0;

    const img = new Image();
    img.decoding = 'async';

    // The element is kept in the result so a decoded neighbour stays in memory.
    img.onload = () => {
      perfLoaded = performance.now();
      let layout: { width: number; height: number } | undefined;
      if (hybrid) {
        const dims = hybridDims(filePath, meta);
        const resolved = resolveHybridLayout(img.naturalWidth, img.naturalHeight, dims.w, dims.h);
        if (resolved && resolved.width > img.naturalWidth * 1.05) {
          layout = resolved;
          hybridLayouts.set(src, { path: filePath, layout, meta });
          while (hybridLayouts.size > 24) {
            const oldest = hybridLayouts.keys().next().value;
            if (oldest === undefined) break;
            hybridLayouts.delete(oldest);
          }
        }
      }
      img.decode()
        .then(() => {
          resolve({
            src,
            naturalWidth: img.naturalWidth,
            naturalHeight: img.naturalHeight,
            layout,
            img,
            perf: { start: perfStart, loaded: perfLoaded, decoded: performance.now() },
          });
        })
        .catch(() => {
          resolve({
            src,
            naturalWidth: img.naturalWidth,
            naturalHeight: img.naturalHeight,
            layout,
            img,
          });
        });
    };

    img.onerror = () => {
      if (!full) preloadCache.delete(filePath);
      reject(new Error(`Error loading image: ${filePath}`));
    };

    if (hybrid) {
      const base = getPreviewUrl(meta?.id ?? props.fileId, filePath, false, meta?.version ?? props.fileVersion);
      if (base) {
        const sharp = Math.min(100, Math.max(0, Number(config.settings.hybridSharpness ?? 40)));
        src = withQuery(base, `fit=${hybridFitEdge()}&sharp=${sharp}&filter=${Math.min(2, Math.max(0, Number(config.settings.hybridFilter ?? 0)))}&cache=${hybridDiskCacheFor(filePath, meta) ? 1 : 0}`);
        img.src = src;
        return;
      }
    }

    if (usesBackendPreviewFor(filePath, Number(props.fileType || 0), meta)) {
      src = getPreviewUrl(
        meta?.id ?? props.fileId,
        filePath,
        false,
        meta?.version ?? props.fileVersion,
      );
      if (!src) {
        if (!full) preloadCache.delete(filePath);
        reject(new Error(`Failed to resolve RAW/TIFF preview source: ${filePath}`));
        return;
      }
      img.src = src;
      return;
    }

    try {
      src = getAssetSrc(filePath, props.fileVersion);
    } catch (error) {
      preloadCache.delete(filePath);
      reject(error);
      return;
    }

    if (!src) {
      preloadCache.delete(filePath);
      reject(new Error('Failed to resolve asset source'));
      return;
    }

    img.src = src;
  });

  if (!full) preloadCache.set(filePath, loadPromise);
  return loadPromise;
}

// ---- Neighbour preloading --------------------------------------------------
// Up to PRELOAD_AHEAD following and PRELOAD_BEHIND previous images are decoded
// and kept in memory (via preloadCache) so stepping through a folder is instant.
// A pixel budget keeps very large photos from using too much RAM.
const PRELOAD_PIXEL_BUDGET = 350_000_000;   // ~1.4 GB of decoded RGBA
let warmImageRequest = 0;

const neighborMeta = new Map<string, { id?: number; version?: number; width?: number; height?: number; albumId?: number }>();
function getNeighborPaths(): string[] {
  const list: Array<string | { path: string; id?: number; version?: number; width?: number; height?: number; album_id?: number }> =
    props.neighborFilePaths.length > 0 ? props.neighborFilePaths : [props.nextFilePath];
  const seen = new Set<string>();
  const out: string[] = [];
  neighborMeta.clear();
  for (const entry of list) {
    const path = typeof entry === 'string' ? entry : entry?.path;
    if (!path || path === props.filePath || seen.has(path)) continue;
    seen.add(path);
    out.push(path);
    if (typeof entry !== 'string') neighborMeta.set(path, { id: entry.id, version: entry.version, width: entry.width, height: entry.height, albumId: entry.album_id });
  }
  return out;
}

// ---- Raster / decode pre-warming ---------------------------------------------
// A big photo shown through a CSS scale() makes the browser decode it and
// rasterise tiles lazily (black screen / thin strip, ~0.5 s delay on the first
// zoom to 100 %). Invisible, almost transparent copies of the image force that
// work to happen early: neighbours at fit size (instant navigation) and the
// current image at 1:1 (instant first zoom to 100 %).
const PRELOAD_BACKEND_COUNT = 6;
const NEIGHBOR_FULL_COUNT = 2;
const RASTER_WARM_MIN_PIXELS = 4_000_000;
const RASTER_WARM_NEIGHBORS = 4;
let rasterWarmHost: HTMLDivElement | null = null;
const rasterWarmFit = new Map<string, HTMLImageElement>();
let rasterWarmZoom: HTMLImageElement | null = null;
let rasterWarmZoomPath = '';
let rasterWarmZoomTimer: ReturnType<typeof setTimeout> | null = null;

function getRasterWarmHost(): HTMLDivElement | null {
  const parent = container.value as HTMLElement | null;
  if (!parent) return null;
  if (!rasterWarmHost || rasterWarmHost.parentElement !== parent) {
    rasterWarmHost = document.createElement('div');
    rasterWarmHost.setAttribute('aria-hidden', 'true');
    rasterWarmHost.style.cssText = 'position:absolute;left:0;top:0;width:100%;height:100%;overflow:hidden;opacity:0.004;pointer-events:none;z-index:-1;';
    parent.appendChild(rasterWarmHost);
  }
  return rasterWarmHost;
}

function makeWarmImage(src: string, width: number, height: number, left: number, top: number) {
  const el = new Image();
  el.decoding = 'async';
  el.alt = '';
  el.draggable = false;
  el.style.cssText = `position:absolute;max-width:none;width:${width}px;height:${height}px;left:${left}px;top:${top}px;`;
  el.src = src;
  return el;
}

function isWarmEligible(loaded: LoadedImage) {
  return !loaded.raw && (loaded.naturalWidth || 0) * (loaded.naturalHeight || 0) >= RASTER_WARM_MIN_PIXELS;
}

function pruneFitWarm(keepPaths: string[]) {
  const keep = new Set(keepPaths);
  for (const [path, el] of rasterWarmFit) {
    if (!keep.has(path)) {
      el.remove();
      rasterWarmFit.delete(path);
    }
  }
}

function warmFitRaster(path: string, loaded: LoadedImage) {
  if (!config.settings.rasterWarm) return;
  if (rasterWarmFit.has(path) || !isWarmEligible(loaded)) return;
  const host = getRasterWarmHost();
  const cw = containerSize.value.width;
  const ch = containerSize.value.height;
  if (!host || cw <= 0 || ch <= 0) return;
  const fit = Math.min(cw / loaded.naturalWidth, ch / loaded.naturalHeight, 1);
  const w = loaded.naturalWidth * fit;
  const h = loaded.naturalHeight * fit;
  const el = makeWarmImage(loaded.src, w, h, (cw - w) / 2, (ch - h) / 2);
  host.appendChild(el);
  rasterWarmFit.set(path, el);
}

function clearZoomWarm() {
  if (rasterWarmZoomTimer) { clearTimeout(rasterWarmZoomTimer); rasterWarmZoomTimer = null; }
  rasterWarmZoom?.remove();
  rasterWarmZoom = null;
  rasterWarmZoomPath = '';
}

function scheduleZoomWarm(path: string, loaded: LoadedImage) {
  if (rasterWarmZoomPath === path && rasterWarmZoom) return;
  clearZoomWarm();
  if (!config.settings.rasterWarm || !isWarmEligible(loaded)) return;
  rasterWarmZoomTimer = setTimeout(() => {
    rasterWarmZoomTimer = null;
    if (path !== props.filePath) return;
    const host = getRasterWarmHost();
    const cw = containerSize.value.width;
    const ch = containerSize.value.height;
    if (!host || cw <= 0 || ch <= 0) return;
    // 1:1 device pixels, centred, so the visible part is rasterised at 100 %.
    const dpr = window.devicePixelRatio || 1;
    const w = loaded.naturalWidth / dpr;
    const h = loaded.naturalHeight / dpr;
    rasterWarmZoom = makeWarmImage(loaded.src, w, h, (cw - w) / 2, (ch - h) / 2);
    rasterWarmZoomPath = path;
    host.appendChild(rasterWarmZoom);
  }, 250);
}

watch(() => config.settings.rasterWarm, (enabled) => {
  if (!enabled) disposeRasterWarm();
});

// Just before a new big photo replaces the current one, a probe that mirrors the
// real display layout (natural-size box scaled to fit) is attached almost
// transparently. Waiting for its decode and a few frames lets the browser do the
// expensive work while the previous photo is still on screen, so the new photo
// appears complete instead of building up from a thin strip.
async function stageForDisplay(loaded: LoadedImage, loadingId: number) {
  if (config.settings.stageBeforeShow === false) return;
  // Backend-made JPEG previews decode quickly and never build up as a strip, so no staging needed.
  if (/^(https?:\/\/preview\.localhost|preview:\/\/)/.test(loaded.src)) return;
  if (!isWarmEligible(loaded)) return;
  const host = getRasterWarmHost();
  const cw = containerSize.value.width;
  const ch = containerSize.value.height;
  if (!host || cw <= 0 || ch <= 0) return;
  const fit = Math.min(cw / loaded.naturalWidth, ch / loaded.naturalHeight, 1);
  const el = new Image();
  el.decoding = 'async';
  el.alt = '';
  el.style.cssText = `position:absolute;max-width:none;left:50%;top:50%;width:${loaded.naturalWidth}px;height:${loaded.naturalHeight}px;margin-left:${-loaded.naturalWidth / 2}px;margin-top:${-loaded.naturalHeight / 2}px;transform:scale(${fit});will-change:transform;backface-visibility:hidden;`;
  el.src = loaded.src;
  host.appendChild(el);
  try {
    await Promise.race([el.decode(), new Promise<void>((resolve) => setTimeout(resolve, 2500))]);
    for (let i = 0; i < 3 && loadingId === currentLoadingId.value; i++) await waitForNextPaint();
  } catch {
    // The real display path surfaces decode errors.
  } finally {
    el.remove();
  }
}

function disposeRasterWarm() {
  clearZoomWarm();
  rasterWarmFit.clear();
  rasterWarmHost?.remove();
  rasterWarmHost = null;
}

// Cloud-only checks are a backend round trip that can queue behind other work
// (it was seen taking 1+ s). A "local" answer is remembered briefly and shared,
// so preloaded neighbours do not pay for it again when they are opened.
const cloudCheckCache = new Map<string, { at: number; promise: Promise<boolean> }>();
const CLOUD_CHECK_TTL_MS = 60_000;
function checkCloudOnlyCached(path: string): Promise<boolean> {
  // Off by default: no backend round trip at all for people who do not use OneDrive-style placeholders.
  if (config.settings.detectCloudFiles !== true) return Promise.resolve(false);
  const now = Date.now();
  const hit = cloudCheckCache.get(path);
  if (hit && now - hit.at < CLOUD_CHECK_TTL_MS) return hit.promise;
  const promise = isCloudOnlyFile(path).then(
    (cloud) => { if (cloud) cloudCheckCache.delete(path); return cloud; },
    (err) => { cloudCheckCache.delete(path); throw err; },
  );
  cloudCheckCache.set(path, { at: now, promise });
  if (cloudCheckCache.size > 64) {
    const oldest = cloudCheckCache.keys().next().value;
    if (oldest !== undefined) cloudCheckCache.delete(oldest);
  }
  return promise;
}

function warmNeighbors() {
  cancelWarmImageScheduling();
  const request = warmImageRequest;
  const paths = getNeighborPaths();
  pruneFitWarm(paths.slice(0, RASTER_WARM_NEIGHBORS));
  if (paths.length === 0) return;

  const isStale = () => request !== warmImageRequest;
  const step = async () => {
    let pixels = 0;
    for (const path of paths) {
      if (isStale()) return;
      if (Number(props.fileType) === 3) return;
      const meta = neighborMeta.get(path);
      // Backend-decoded formats (PSD, TIFF ...) are expensive to produce, so only
      // the nearest few are prepared, and only when their file id is known.
      if (usesBackendPreviewFor(path, 1, meta) && (!meta?.id || paths.indexOf(path) >= PRELOAD_BACKEND_COUNT)) continue;
      try {
        // Reading a cloud-only original would trigger a download.
        if (await checkCloudOnlyCached(path)) continue;
        if (isStale()) return;
        const loaded = await loadImageResource(path, meta);
        pixels += (loaded.naturalWidth || 0) * (loaded.naturalHeight || 0);
        if (!isStale() && paths.indexOf(path) < RASTER_WARM_NEIGHBORS) warmFitRaster(path, loaded);
        // The nearest neighbours also get their full resolution prepared, so a drag to 100 % right
        // after stepping to them does not show the enlarged preview first.
        if (!isStale() && paths.indexOf(path) < NEIGHBOR_FULL_COUNT && isHybridCandidate(path, meta)
          && hybridLayouts.has(loaded.src) && !isHugeFull({ path, meta })) {
          void loadHybridFull({ path, meta } as HybridInfo).catch(() => {});
        }
      } catch {
        // Ignore preload failures and let the main load path surface errors.
      }
      if (pixels > PRELOAD_PIXEL_BUDGET) return;
      // Yield between images so the UI thread is never starved.
      await new Promise<void>((resolve) => {
        if (typeof window !== 'undefined' && 'requestIdleCallback' in window) {
          warmImageIdleId = window.requestIdleCallback(() => { warmImageIdleId = null; resolve(); }, { timeout: 300 });
        } else {
          setTimeout(resolve, 0);
        }
      });
    }
  };

  warmImageTimeout = setTimeout(() => {
    warmImageTimeout = null;
    if (isStale()) return;
    void step();
  }, 150);
}

function cancelWarmImageScheduling() {
  warmImageRequest++;
  if (warmImageTimeout) {
    clearTimeout(warmImageTimeout);
    warmImageTimeout = null;
  }
  if (warmImageIdleId !== null && typeof window !== 'undefined' && 'cancelIdleCallback' in window) {
    window.cancelIdleCallback(warmImageIdleId);
    warmImageIdleId = null;
  }
}

function clearStalePreloadEntries(activeFilePath?: string, neighborPaths: string[] = []) {
  const keep = new Set<string>(neighborPaths);
  if (activeFilePath) keep.add(activeFilePath);

  for (const key of preloadCache.keys()) {
    if (!keep.has(key)) {
      preloadCache.delete(key);
    }
  }

}

function loadPlaceholderResource(src?: string) {
  return new Promise<{ src: string; naturalWidth: number; naturalHeight: number }>((resolve, reject) => {
    if (!src) {
      reject(new Error('Missing placeholder src'));
      return;
    }

    const img = new Image();
    img.decoding = 'async';
    img.onload = () => {
      img.decode()
        .then(() => resolve({ src, naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight }))
        .catch(() => resolve({ src, naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight }));
    };
    img.onerror = () => reject(new Error('Failed to load placeholder image'));
    img.src = src;
  });
}

function getCompatibleLayout(
  naturalWidth: number,
  naturalHeight: number,
  preferredWidth: number,
  preferredHeight: number,
) {
  if (!preferredWidth || !preferredHeight || !naturalWidth || !naturalHeight) {
    return { width: naturalWidth, height: naturalHeight };
  }

  const naturalRatio = naturalWidth / naturalHeight;
  const preferredRatio = preferredWidth / preferredHeight;
  return Math.abs(naturalRatio - preferredRatio) / naturalRatio <= 0.01
    ? { width: preferredWidth, height: preferredHeight }
    : { width: naturalWidth, height: naturalHeight };
}

function setImageSlot(
  slotIndex: number,
  filePath: string,
  src: string,
  naturalWidth: number,
  naturalHeight: number,
  layoutWidth: number = naturalWidth,
  layoutHeight: number = naturalHeight,
) {
  const hybridEntry = hybridLayouts.get(src);
  if (hybridEntry && hybridEntry.path === filePath) {
    layoutWidth = hybridEntry.layout.width;
    layoutHeight = hybridEntry.layout.height;
  }
  markHybridSlot(slotIndex, filePath, src, naturalWidth);
  imageNaturalSize.value[slotIndex] = { width: naturalWidth, height: naturalHeight };
  if (imageSrc.value[slotIndex] !== src) slotDecoded.value[slotIndex] = false;
  imageSrc.value[slotIndex] = src;
  imageFilePath.value[slotIndex] = filePath;
  imageRotate.value[slotIndex] = props.rotate;
  imageSize.value[slotIndex] = {
    width: layoutWidth,
    height: layoutHeight,
  };

  if (Math.abs(props.rotate % 180) === 90) {
    imageSizeRotated.value[slotIndex] = {
      width: layoutHeight,
      height: layoutWidth,
    };
  } else {
    imageSizeRotated.value[slotIndex] = {
      width: layoutWidth,
      height: layoutHeight,
    };
  }
}

const isDraggingNavBox = ref(false);
const navigatorViewport = computed(() => ({
  ...getViewportState(),
  viewportWidth: containerSize.value.width,
  viewportHeight: containerSize.value.height,
  rotate: imageRotate.value[activeImage.value],
  pannable: isGrabbing.value,
}));
function navigateImage(point: { normX: number; normY: number }) {
  noTransition.value = true;
  applyViewportState({ ...getViewportState(), ...point });
  requestAnimationFrame(() => { noTransition.value = false; });
}
function zoomNavigator(factor: number) {
  const size = containerSize.value;
  zoomImage(size.width / 2, size.height / 2,
    Math.min(maxScale.value, Math.max(minScale.value, scale.value[activeImage.value] * factor)));
}

onMounted(() => {
  // observe container size changes
  resizeObserver = new ResizeObserver(entries => {
    for (let entry of entries) {
      containerSize.value = {
        width: entry.contentRect.width,
        height: entry.contentRect.height,
      };
      mousePosition.value = { x: entry.contentRect.width / 2, y: entry.contentRect.height / 2 };
    }
  });

  if (container.value) {
    resizeObserver.observe(container.value);  // Observe container size changes
    updatePosition();   // Initial position calculation
    const el = container.value as HTMLElement;
    el.addEventListener('pointerdown', handlePinchPointerDown);
    el.addEventListener('pointermove', handlePinchPointerMove, { passive: false });
    el.addEventListener('pointerup', handlePinchPointerEnd);
    el.addEventListener('pointercancel', handlePinchPointerEnd);
    el.addEventListener('pointerleave', handlePinchPointerEnd);
  }
  // Global capture-phase fallback: in WebView2 some touchpad pinch events can
  // be intercepted before they bubble to the container element. Listening at
  // window with capture+passive:false guarantees we see them and can preventDefault.
  window.addEventListener('wheel', handleGlobalPinchWheel, { capture: true, passive: false });
});

onBeforeUnmount(() => {
  disposeRasterWarm();
  document.removeEventListener('mousemove', trackImageDragOut, true);
  document.documentElement.removeEventListener('mouseleave', trackImageDragOut);
  document.removeEventListener('mouseup', handleImageMouseUp, true);
  endLoupe();
  rawAbortController?.abort();
  for (const url of rawObjectUrls) URL.revokeObjectURL(url);
  if (rawUpgradeTimer) clearTimeout(rawUpgradeTimer);
  rawObjectUrls.clear();
  cancelAnimationFrame(resizeTransitionFrame);
  if (debounceTimeout) clearTimeout(debounceTimeout);
  if (rawLoadingTimer) clearTimeout(rawLoadingTimer);
  if (resizeObserver && container.value) {
    resizeObserver.unobserve(container.value);
    resizeObserver.disconnect();
  }
  if (container.value) {
    const el = container.value as HTMLElement;
    el.removeEventListener('pointerdown', handlePinchPointerDown);
    el.removeEventListener('pointermove', handlePinchPointerMove);
    el.removeEventListener('pointerup', handlePinchPointerEnd);
    el.removeEventListener('pointercancel', handlePinchPointerEnd);
    el.removeEventListener('pointerleave', handlePinchPointerEnd);
  }
  window.removeEventListener('wheel', handleGlobalPinchWheel, { capture: true });
  if (loadingTimeout) { // Clear timeout on unmount
    clearTimeout(loadingTimeout);
  }
  if (navigatorAutoHideTimer) {
    clearTimeout(navigatorAutoHideTimer);
  }
  cancelWarmImageScheduling();
});

function handleGlobalPinchWheel(event: WheelEvent) {
  if (!event.ctrlKey) return;
  if (!container.value) return;
  const rect = (container.value as HTMLElement).getBoundingClientRect();
  if (
    event.clientX < rect.left || event.clientX > rect.right ||
    event.clientY < rect.top || event.clientY > rect.bottom
  ) return;
  event.preventDefault();
  event.stopPropagation();
  isWheelZooming.value = true;
  if (wheelZoomTimeout) clearTimeout(wheelZoomTimeout);
  wheelZoomTimeout = setTimeout(() => { isWheelZooming.value = false; }, 200);
  applyZoomFromWheel(event);
}

// Touchpad pinch is dispatched by Chromium as small-delta ctrl+wheel events
// (deltaY = -log(scale_change) * 96), while Ctrl+mouse-wheel sends deltaY ~100
// per notch. Use the browser-matching exp formula for pinch and the gentler
// notch-based wheelZoom for mouse so each input feels native.
function applyZoomFromWheel(event: WheelEvent) {
  const isPinch = event.ctrlKey && event.deltaMode === 0 && Math.abs(event.deltaY) < 50;
  if (!isPinch) {
    wheelZoom(event, 1);
    return;
  }
  const currentScale = scale.value[activeImage.value];
  let newScale = currentScale * Math.exp(-event.deltaY / 96);
  newScale = Math.min(Math.max(newScale, minScale.value), maxScale.value);
  if (container.value) {
    const rect = (container.value as HTMLElement).getBoundingClientRect();
    const containerSizeVal = containerSize.value;
    const x = ((event.clientX - rect.left) * containerSizeVal.width) / rect.width;
    const y = ((event.clientY - rect.top) * containerSizeVal.height) / rect.height;
    zoomImage(x, y, newScale);
  }
}

function handlePinchPointerDown(event: PointerEvent) {
  if (event.pointerType !== 'touch') return;
  activeTouchPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  isTouchActive.value = true;
  if (activeTouchPointers.size === 1) {
    // Single-finger pan: track this pointer. Mark isDraggingImage so the
    // 0.3s CSS transform transition is suppressed and the image follows the
    // finger in real time (the existing mouse drag relies on the same flag).
    panPointerId = event.pointerId;
    panLastPos = { x: event.clientX, y: event.clientY };
    touchSwipeStart = { x: event.clientX, y: event.clientY };
    touchSwipeTriggered = false;
    isDraggingImage.value = true;
  } else if (activeTouchPointers.size === 2) {
    // Promote to pinch: cancel any in-flight pan
    panPointerId = null;
    const pts = Array.from(activeTouchPointers.values());
    pinchStartDistance = Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
    pinchStartScale = scale.value[activeImage.value];
    pinchCenter = {
      x: (pts[0].x + pts[1].x) / 2,
      y: (pts[0].y + pts[1].y) / 2,
    };
    isPinching = true;
    isDraggingImage.value = false;
    // Suppress CSS transitions so the image tracks fingers in real time.
    isWheelZooming.value = true;
    if (wheelZoomTimeout) {
      clearTimeout(wheelZoomTimeout);
      wheelZoomTimeout = null;
    }
  }
}

function handlePinchPointerMove(event: PointerEvent) {
  if (event.pointerType !== 'touch') return;
  if (!activeTouchPointers.has(event.pointerId)) return;
  activeTouchPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });

  // Pinch path
  if (isPinching && activeTouchPointers.size === 2 && pinchStartDistance > 0) {
    event.preventDefault();
    const pts = Array.from(activeTouchPointers.values());
    const distance = Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
    if (distance < 1) return;
    let newScale = pinchStartScale * (distance / pinchStartDistance);
    newScale = Math.min(Math.max(newScale, minScale.value), maxScale.value);
    if (container.value) {
      const rect = (container.value as HTMLElement).getBoundingClientRect();
      const containerSizeVal = containerSize.value;
      const x = ((pinchCenter.x - rect.left) * containerSizeVal.width) / rect.width;
      const y = ((pinchCenter.y - rect.top) * containerSizeVal.height) / rect.height;
      zoomImage(x, y, newScale);
    }
    return;
  }

  // Single-finger path: pan when the image is bigger than the container,
  // otherwise interpret horizontal motion as a swipe to navigate prev/next.
  if (panPointerId === event.pointerId && activeTouchPointers.size === 1) {
    event.preventDefault();
    const imgIndex = activeImage.value;

    if (!isGrabbing.value && !props.isSlideShow) {
      if (touchSwipeTriggered) return;
      const totalX = event.clientX - touchSwipeStart.x;
      const totalY = event.clientY - touchSwipeStart.y;
      const absX = Math.abs(totalX);
      const absY = Math.abs(totalY);
      if (absX >= MOUSE_DRAG_NAV_THRESHOLD && absX > absY) {
        const direction = totalX < 0 ? 'next' : 'prev';
        navDirection.value = direction;
        emit('message-from-image-viewer', { message: direction });
        touchSwipeTriggered = true;
        isDraggingImage.value = false;
      }
      return;
    }

    const scaleVal = scale.value[imgIndex];
    const imgRotatedSize = imageSizeRotated.value[imgIndex];
    const containerSizeVal = containerSize.value;
    const scaledWidth = imgRotatedSize.width * scaleVal;
    const scaledHeight = imgRotatedSize.height * scaleVal;
    const dx = scaledWidth <= containerSizeVal.width ? 0 : event.clientX - panLastPos.x;
    const dy = scaledHeight <= containerSizeVal.height ? 0 : event.clientY - panLastPos.y;
    panLastPos = { x: event.clientX, y: event.clientY };
    if (dx === 0 && dy === 0) return;
    position.value[imgIndex].x += dx;
    position.value[imgIndex].y += dy;
    clampPosition();
  }
}

function handlePinchPointerEnd(event: PointerEvent) {
  if (event.pointerType !== 'touch') return;
  activeTouchPointers.delete(event.pointerId);
  if (event.pointerId === panPointerId) panPointerId = null;
  if (activeTouchPointers.size < 2) {
    pinchStartDistance = 0;
  }
  if (activeTouchPointers.size === 0) {
    isPinching = false;
    isTouchActive.value = false;
    isDraggingImage.value = false;
    if (wheelZoomTimeout) clearTimeout(wheelZoomTimeout);
    wheelZoomTimeout = setTimeout(() => {
      isWheelZooming.value = false;
    }, 50);
  }
}

const updatePosition = () => {
  if (container.value) {
    const rect = (container.value as HTMLElement).getBoundingClientRect();
    containerPos.value = { x: rect.left, y: rect.top };
  }
};

// Preloaded next images must not retain a different RAW display policy.
watch(rawDisplayKey, () => { preloadCache.clear(); fullCache.clear(); fullPixels.clear(); hybridLayouts.clear(); }, { flush: 'sync' });
watch(() => [config.settings.hybridPreview, config.settings.hybridSharpness, config.settings.hybridFilter], () => { preloadCache.clear(); fullCache.clear(); fullPixels.clear(); hybridLayouts.clear(); });

async function showOfflinePreview(filePath: string, loadingId: number) {
  if (loadingId !== currentLoadingId.value) return;
  if (loadingTimeout) { clearTimeout(loadingTimeout); loadingTimeout = null; }
  isLoading.value = false;
  rawRequestPending.value = false;
  if (imageFilePath.value[activeImage.value] === filePath && imageSrc.value[activeImage.value]) {
    offlinePreview.value = true;
    const size = imageNaturalSize.value[activeImage.value];
    maxScale.value = Math.max(scale.value[activeImage.value], Math.min(1, size.width / (imageSize.value[activeImage.value].width || 1)));
    return;
  }
  let loaded: LoadedImage | null = null;
  try {
    const cached = await preloadCache.get(filePath);
    if (cached) loaded = await loadPlaceholderResource(cached.src);
  } catch { /* Try thumbnail next. */ }
  if (!loaded && props.fileId) {
    const preview = getPreviewUrl(props.fileId, filePath, false, props.fileVersion);
    try { loaded = await loadPlaceholderResource(`${preview}${preview.includes('?') ? '&' : '?'}cachedOnly=true`); } catch { /* Try thumbnail next. */ }
  }
  if (!loaded) {
    try { loaded = await loadPlaceholderResource(await getEffectiveThumbnailSrc()); } catch { /* No local preview. */ }
  }
  if (loadingId !== currentLoadingId.value) return;
  offlinePreview.value = true;
  if (!loaded) {
    imageSrc.value = ['', ''];
    imageFilePath.value = ['', ''];
    loadError.value = true;
    return;
  }
  const slot = activeImage.value ^ 1;
  setImageSlot(slot, filePath, loaded.src, loaded.naturalWidth, loaded.naturalHeight);
  scale.value[slot] = 1;
  position.value[slot] = { x: 0, y: 0 };
  isZoomFit.value = true;
  maxScale.value = 1;
  onImageReady(slot);
}

// Watch file changes and the selected RAW preview source.
watch([
  () => props.filePath,
  () => props.fileVersion,
  () => Number(props.fileType || 0) === 3 ? `${rawDisplayKey()}:${rawSelectionVersion.value}:${JSON.stringify(rawOverride.value)}` : '',
  () => props.originalUnavailable,
], async ([newFilePath, newFileVersion, newRawThumbnailSource], [oldFilePath, oldFileVersion, oldRawThumbnailSource, oldUnavailable]) => {
  // Cancel previous loading
  if (rawUpgradeTimer) { clearTimeout(rawUpgradeTimer); rawUpgradeTimer = null; }
  rawAbortController?.abort();
  currentLoadingId.value++;
  const loadingId = currentLoadingId.value;
  cancelWarmImageScheduling();
  if (rasterWarmZoomPath !== String(newFilePath || '')) clearZoomWarm();
  if (
    newFilePath
    && newFilePath === oldFilePath
    && (newFileVersion !== oldFileVersion || newRawThumbnailSource !== oldRawThumbnailSource || (oldUnavailable && !props.originalUnavailable))
  ) {
    preloadCache.delete(newFilePath);
  }
  clearStalePreloadEntries(newFilePath || '', getNeighborPaths());

  if (loadingTimeout) {
    clearTimeout(loadingTimeout);
    loadingTimeout = null;
  }

  loadError.value = false; // Reset error state
  cloudLoadError.value = false;
  offlinePreview.value = false;
  maxScale.value = 10;

  if (!newFilePath) {
    isLoading.value = false;
    return;
  }

  if (props.originalUnavailable) {
    await showOfflinePreview(String(newFilePath), loadingId);
    return;
  }
  if (Number(props.fileType) === 3) isLoading.value = true;

  // Set timeout to show loading overlay if loading takes too long
  loadingTimeout = setTimeout(() => {
    isLoading.value = true;
  }, 500);

  const perfT0 = performance.now();
  const wasPreloaded = preloadCache.has(String(newFilePath));
  let perfCloud = perfT0;
  const usesBackendPreview = usesBackendPreviewFor(String(newFilePath), Number(props.fileType || 0));
  const ffmpegExtensionsPromise = getFfmpegBackedPreviewExtensions();
  const isRawPreview = Number(props.fileType || 0) === 3;
  const requested = requestedRawOptions.value;
  const attemptedMode: RawMode | null = isRawPreview && !requested.preferPair
    ? requested.mode === 'embedded' ? 'embedded' : requested.autoBright ? 'brightened' : 'rendered'
    : null;

  try {
    const usesRealtimePreview = (await ffmpegExtensionsPromise).has(getFileExtension(newFilePath).toLowerCase());
    if (loadingId !== currentLoadingId.value) return;
    const cloudOnly = await checkCloudOnlyCached(newFilePath);
    perfCloud = performance.now();
    if (loadingId !== currentLoadingId.value) return;
    if (cloudOnly) {
      clearTimeout(loadingTimeout);
      loadingTimeout = null;
      isLoading.value = false;
      rawRequestPending.value = false;
      cloudLoadError.value = true;
      loadError.value = true;
      return;
    }
    const imageResultPromise = loadImageResource(newFilePath)
      .then((loaded) => ({ kind: 'image' as const, loaded }));
    const keepCurrentRaw = isRawPreview && newFilePath === oldFilePath && !!rawSource.value;
    const thumbnailResultPromise = !keepCurrentRaw && (!isRawPreview || requestedRawOptions.value.mode === 'embedded' || requestedRawOptions.value.preferPair) && !usesRealtimePreview && ((usesBackendPreview && !isBackendPngPath(String(newFilePath))) || props.showThumbnailPlaceholder)
      ? getEffectiveThumbnailSrc()
        .then(async (src) => {
          if (!src) return { kind: 'thumbnail' as const, placeholder: null };
          try {
            return { kind: 'thumbnail' as const, placeholder: await loadPlaceholderResource(src) };
          } catch {
            return { kind: 'thumbnail' as const, placeholder: null };
          }
        })
        .catch(() => ({ kind: 'thumbnail' as const, placeholder: null }))
      : Promise.resolve({ kind: 'thumbnail' as const, placeholder: null });
    const firstResult = await Promise.race([imageResultPromise, thumbnailResultPromise]);
    let hasPreviewPlaceholder = false;

    if (firstResult.kind === 'thumbnail' && firstResult.placeholder) {
      if (loadingId !== currentLoadingId.value) return;
      const nextImageIndex = activeImage.value ^ 1;
      const layout = getCompatibleLayout(
        firstResult.placeholder.naturalWidth,
        firstResult.placeholder.naturalHeight,
        props.imageWidth,
        props.imageHeight,
      );
      setImageSlot(
        nextImageIndex,
        newFilePath,
        firstResult.placeholder.src,
        firstResult.placeholder.naturalWidth,
        firstResult.placeholder.naturalHeight,
        layout.width,
        layout.height,
      );
      onImageReady(nextImageIndex, true);
      hasPreviewPlaceholder = true;
      await nextTick();
      await waitForNextPaint();
    }

    const loaded = firstResult.kind === 'image'
      ? firstResult.loaded
      : (await imageResultPromise).loaded;
    if (loadingId !== currentLoadingId.value) {
      if (loaded.raw) { URL.revokeObjectURL(loaded.src); rawObjectUrls.delete(loaded.src); }
      return;
    }

    const perfStageStart = performance.now();
    await stageForDisplay(loaded, loadingId);
    const perfStage = performance.now() - perfStageStart;
    if (loadingId !== currentLoadingId.value) {
      if (loaded.raw) { URL.revokeObjectURL(loaded.src); rawObjectUrls.delete(loaded.src); }
      return;
    }

    if (loadingTimeout) {
      clearTimeout(loadingTimeout);
      loadingTimeout = null;
    }
    isLoading.value = false;

    nextTick(() => {
      if (loadingId !== currentLoadingId.value) {
        if (loaded.raw) { URL.revokeObjectURL(loaded.src); rawObjectUrls.delete(loaded.src); }
        return;
      }
      if (loaded.raw) {
        rawRequestPending.value = false;
        rawSource.value = loaded.raw.source;
        // RAW-only requests do not query companions; retain the known pair.
        if (loaded.raw.pair !== null) resolvedRawPairLabel.value = loaded.raw.pair;
        rawEmbeddedUnavailable.value ||= loaded.raw.unavailable;
        if (attemptedMode && loaded.raw.source !== attemptedMode && loaded.raw.source !== 'image') {
          failedRawModes.value.add(attemptedMode);
        }
      }
      isZoomFit.value = props.isZoomFit;
      const activeIndex = activeImage.value;
      const showingPlaceholderForCurrentFile = hasPreviewPlaceholder
        && imageFilePath.value[activeIndex] === newFilePath;

      if (showingPlaceholderForCurrentFile) {
        noTransition.value = true;
        // Reuse the placeholder layout so replacing source pixels does not
        // change the displayed geometry. In particular, RAW thumbnails use
        // the complete image dimensions, preserving their scale on replacement.
        const placeholderLayout = getCompatibleLayout(
          loaded.naturalWidth,
          loaded.naturalHeight,
          imageSize.value[activeIndex].width,
          imageSize.value[activeIndex].height,
        );
        setImageSlot(
          activeIndex,
          newFilePath,
          loaded.src,
          loaded.naturalWidth,
          loaded.naturalHeight,
          placeholderLayout.width,
          placeholderLayout.height,
        );

        if (containerSize.value.width > 0 && isZoomFit.value) {
          // Orientation/aspect differences can change the placeholder layout.
          updateZoomFit(true);
        } else if (isRawPreview && containerSize.value.width > 0) {
          clampPosition(true);
        }

        setTimeout(() => {
          noTransition.value = false;
        }, 150);
      } else {
        const nextImageIndex = activeIndex ^ 1;
        scale.value[nextImageIndex] = 1;
        position.value[nextImageIndex] = { x: 0, y: 0 };
        const layout = isRawPreview
          ? getCompatibleLayout(loaded.naturalWidth, loaded.naturalHeight, props.imageWidth, props.imageHeight)
          : { width: loaded.naturalWidth, height: loaded.naturalHeight };
        setImageSlot(
          nextImageIndex,
          newFilePath,
          loaded.src,
          loaded.naturalWidth,
          loaded.naturalHeight,
          layout.width,
          layout.height,
        );
        onImageReady(nextImageIndex);
      }
      const perfSlot = performance.now();
      requestAnimationFrame(() => requestAnimationFrame(() => {
        if (loadingId === currentLoadingId.value) {
          scheduleZoomWarm(String(newFilePath), loaded);
          reportLoadTiming(String(newFilePath), perfT0, perfCloud, loaded, perfSlot, performance.now(), wasPreloaded, perfStage);
        }
      }));
      for (const url of rawObjectUrls) {
        if (!imageSrc.value.includes(url)) { URL.revokeObjectURL(url); rawObjectUrls.delete(url); }
      }
      warmNeighbors();
    });
  } catch (e) {
    console.error("Error getting asset source:", e);
    if (loadingId !== currentLoadingId.value) return;
    if (loadingTimeout) {
      clearTimeout(loadingTimeout);
      loadingTimeout = null;
    }
    isLoading.value = false;
    rawRequestPending.value = false;
    const cloudOnly = await isCloudOnlyFile(newFilePath);
    if (loadingId !== currentLoadingId.value) return;
    if (cloudOnly) {
      cloudLoadError.value = true;
      loadError.value = true;
      return;
    }
    try {
      if (!await checkFileAccessibility(String(newFilePath))) {
        if (loadingId !== currentLoadingId.value) return;
        setFileAccessibility(String(newFilePath), false);
        await showOfflinePreview(String(newFilePath), loadingId);
        return;
      }
    } catch { /* Keep the decoding error if access could not be checked. */ }
    if (loadingId !== currentLoadingId.value) return;
    // Keep the displayed RAW preview and allow retrying a failed request.
    if (!isRawPreview || newFilePath !== oldFilePath || !rawSource.value) {
      loadError.value = true;
    }
  }
}, { immediate: true });

watch(() => props.fileId, () => {
  resolvedThumbnailSrc.value = '';
  resolvedThumbnailFileId.value = 0;
});

// watch thumbnail source changes to update placeholder if original is still loading
watch(displayThumbnailSrc, async (newThumbSrc) => {
  if (props.originalUnavailable || offlinePreview.value) return;
  if (Number(props.fileType) === 3) return;
  if (!newThumbSrc) return;
  const currentFilePath = props.filePath;
  if (!currentFilePath) return;
  const loadingId = currentLoadingId.value;

  const usesBackendPreview = usesBackendPreviewFor(currentFilePath, Number(props.fileType || 0));
  const ffmpegExtensions = await getFfmpegBackedPreviewExtensions();
  if (!usesBackendPreview || ffmpegExtensions.has(getFileExtension(currentFilePath).toLowerCase())) return;

  // Only update if we are still waiting for the full image OR if we are currently showing a stale placeholder
  const activeIndex = activeImage.value;
  
  // We check if it's the full original image by checking the src. 
  // For backend preview, the full image src is from getPreviewUrl.
  const fullImageSrc = getPreviewUrl(
    props.fileId,
    currentFilePath,
    false,
    props.fileVersion,
  );
  const isCurrentlyShowingFullImage = imageSrc.value[activeIndex] === fullImageSrc;
  
  if (isCurrentlyShowingFullImage) return;

  try {
    const placeholder = await loadPlaceholderResource(newThumbSrc);
    // Never downgrade a full image that finished loading while the placeholder was decoding.
    if (loadingId !== currentLoadingId.value || offlinePreview.value || imageSrc.value.includes(fullImageSrc)) return;
    const layout = getCompatibleLayout(
      placeholder.naturalWidth,
      placeholder.naturalHeight,
      props.imageWidth,
      props.imageHeight,
    );
    // Check if we haven't switched files since we started loading the placeholder
    if (props.filePath === currentFilePath) {
      // If we are currently showing a placeholder for this file, just update it in place
      if (imageFilePath.value[activeIndex] === currentFilePath) {
        noTransition.value = true;
        setImageSlot(
          activeIndex,
          currentFilePath,
          placeholder.src,
          placeholder.naturalWidth,
          placeholder.naturalHeight,
          layout.width,
          layout.height,
        );
        // Important: update layout after size change
        if (isZoomFit.value) {
          updateZoomFit(true);
        } else {
          clampPosition(true);
        }
        setTimeout(() => { noTransition.value = false; }, 150);
      } else {
        // We might be in a slot transition, but showing the other slot's placeholder?
        // Let's just update the target slot if it's assigned to this file
        const targetIndex = imageFilePath.value[0] === currentFilePath ? 0 : (imageFilePath.value[1] === currentFilePath ? 1 : -1);
        if (targetIndex !== -1) {
          setImageSlot(
            targetIndex,
            currentFilePath,
            placeholder.src,
            placeholder.naturalWidth,
            placeholder.naturalHeight,
            layout.width,
            layout.height,
          );
        }
      }
    }
  } catch {
    // ignore
  }
});

// watch fileId / face toggle changes to fetch faces
watch(() => [props.fileId, config.settings.face.enabled], async ([newFileId, faceEnabled]) => {
  faces.value = []; // Clear previous faces
  if (faceEnabled && newFileId) {
    const result = await getFacesForFile(newFileId);
    if (result && result.length > 0) {
      // Parse bbox JSON string for each face
      faces.value = result.map((face: RawFace) => {
        try {
          return {
            ...face,
            bbox: JSON.parse(face.bbox)
          };
        } catch (e) {
          console.error("Error parsing face bbox", e);
          return null;
        }
      }).filter((f: Face | null) => f);
    }
  }
}, { immediate: true });

// watch rotate changes
watch(() => props.rotate, (newRotate) => {
  const activeIndex = activeImage.value;
  const inactiveIndex = activeIndex ^ 1;
  const currentFilePath = props.filePath || '';

  // Update only the buffer(s) that actually render the current file.
  // This avoids mutating the leaving slide during navigation.
  if (imageFilePath.value[activeIndex] === currentFilePath) {
    imageRotate.value[activeIndex] = newRotate;
  }
  if (imageFilePath.value[inactiveIndex] === currentFilePath) {
    imageRotate.value[inactiveIndex] = newRotate;
  }
});

watch(() => imageRotate.value[activeImage.value], (newValue) => {
  const imgIndex = activeImage.value;
  const imgSize = imageSize.value[imgIndex];
  
  // swap image width and height
  if (Math.abs(newValue % 180) === 90) {
    imageSizeRotated.value[imgIndex] = { 
      width: imgSize.height, 
      height: imgSize.width 
    };
  } else {
    imageSizeRotated.value[imgIndex] = { 
      width: imgSize.width,  
      height: imgSize.height 
    };
  }

  if (isZoomFit.value) {
    zoomFit();
  } else {
    clampPosition();
  }
});

// display zoom scale for a while
watch(() => scale.value[activeImage.value], (newValue) => {
  emit('scale', { 
    scale: newValue, 
    displayScale: getDisplayScale(newValue),
    minScale: minScale.value, 
    maxScale: maxScale.value 
  });
});

function resetNavigatorAutoHide() {
  if (navigatorAutoHideTimer) {
    clearTimeout(navigatorAutoHideTimer);
    navigatorAutoHideTimer = null;
  }
  navigatorAutoVisible.value = true;
  if (config.settings.navigatorViewMode === 0) {
    navigatorAutoHideTimer = setTimeout(() => {
      navigatorAutoVisible.value = false;
      navigatorAutoHideTimer = null;
    }, 3000);
  }
}

function pauseNavigatorAutoHide() {
  if (navigatorAutoHideTimer) {
    clearTimeout(navigatorAutoHideTimer);
    navigatorAutoHideTimer = null;
  }
}

watch(
  () => [
    config.settings.navigatorViewMode,
    scale.value[activeImage.value],
    position.value[activeImage.value].x,
    position.value[activeImage.value].y,
  ],
  resetNavigatorAutoHide,
  { immediate: true }
);

// watch zoom fit changes
watch(() => props.isZoomFit, (newValue) => {
  isZoomFit.value = newValue;
  if (skipNextZoomFitWatch) {
    // The change was caused by restoring a saved viewport; keep that viewport.
    skipNextZoomFitWatch = false;
    return;
  }
  // The user took manual control of zoom: forget the pre-restore state.
  zoomFitBeforeRestore = null;
  updateZoomFit();
});

// A resize changes the coordinate system, not the user's zoom intent. Apply
// its fit/position before paint instead of displaying the old layout for 100ms
// and then animating from that stale position (especially after Teleport).
watch(containerSize, (size) => {
  if (size.width <= 0 || size.height <= 0) return;
  const image = imageSizeRotated.value[activeImage.value];
  if (image.width <= 0 || image.height <= 0) return;
  isResizingContainer.value = true;
  cancelAnimationFrame(resizeTransitionFrame);
  if (isZoomFit.value) zoomFit();
  else clampPosition();
  // Keep transitions disabled through a painted frame, including consecutive
  // resize notifications from the native fullscreen animation.
  resizeTransitionFrame = requestAnimationFrame(() => {
    resizeTransitionFrame = requestAnimationFrame(() => {
      isResizingContainer.value = false;
    });
  });
}, { flush: 'sync' });

// Image loading retains its existing deferred layout update.
let debounceTimeout: NodeJS.Timeout | null = null;
watch(() => imageSize.value, () => {
  if (debounceTimeout) clearTimeout(debounceTimeout);
  debounceTimeout = setTimeout(() => {
    if (isZoomFit.value) {
      zoomFit();
    } else {
      clampPosition();
    }
  }, 100); // Debounce for 100ms
});

// Called when the new image is fully loaded and ready to be shown
const onImageReady = (nextIndex: number, preserveLoading: boolean = false) => {
  // A placeholder is ready before the full image. It needs the same viewport
  // initialization, but must not dismiss the full-image loading indicator.
  if (!preserveLoading) {
    if (loadingTimeout) {
      clearTimeout(loadingTimeout);
      loadingTimeout = null;
    }
    isLoading.value = false;
  }
  noTransition.value = true;
  // A viewport belongs to the image content, not to its pixel dimensions.
  // Capture it before switching buffers so the next image can convert the
  // normalized viewport to its own rendered position.
  const previousViewport = getViewportState();
  const previousImageIndex = activeImage.value;
  const hasPreviousImage = imageSize.value[previousImageIndex].width > 0
    && imageSize.value[previousImageIndex].height > 0;
  // Transitioning active image
  activeImage.value = nextIndex;

  const applyZoom = () => {
    // A viewport saved for this file (zoom + position) wins over the current
    // zoom mode and over carrying the previous image's viewport across.
    const savedViewport = getSavedViewport(imageFilePath.value[activeImage.value]);
    setHasSavedViewport(!!savedViewport);
    let leavingSavedViewport = false;
    if (savedViewport && restoreSavedViewport(savedViewport)) {
      setTimeout(() => {
        noTransition.value = false;
      }, 500);
      return;
    }
    if (!savedViewport && props.allowViewportFollow && followViewport.value && applyFollowedViewport()) {
      setTimeout(() => {
        noTransition.value = false;
      }, 500);
      return;
    }
    if (!savedViewport && zoomFitBeforeRestore !== null) {
      // The previous image was shown from a saved viewport and this one has
      // none: go back to the zoom mode that was active before the restore.
      leavingSavedViewport = true;
      const wasFit = zoomFitBeforeRestore;
      zoomFitBeforeRestore = null;
      if (wasFit) {
        isZoomFit.value = true;
        if (!props.isZoomFit) emit('update:isZoomFit', true);
      }
    }
    // If not in "zoom to fit" mode (or zoom is locked), perform a calculated
    // zoom to the cursor position / carry the previous viewport over.
    const zoomLockCarry = zoomLocked.value && hasPreviousImage;
    if (!isZoomFit.value || zoomLockCarry) {
      const imgIndex = activeImage.value;
      const imgSize = imageSize.value[imgIndex];
      const container = containerSize.value;
      if (hasPreviousImage && (zoomLockCarry || !leavingSavedViewport)) {
        // Never carry over absolute x/y offsets: they describe the previous
        // image's dimensions. Rebuild the position from normalized viewport
        // coordinates so image navigation and thumbnail replacement retain
        // the same relative point in the image.
        applyViewportState(previousViewport, true);
        // The silent restore avoids sync feedback, but consumers still need
        // the new file's dimensions and identity after the buffer switch.
        emit('viewport-change', getViewportState());
      } else {
        // Original logic: reset to center or zoom to cursor
        
        // Use current mouse position relative to container
        const cursorX = mousePosition.value.x - containerPos.value.x;
        const cursorY = mousePosition.value.y - containerPos.value.y;

        // Calculate a conceptual "before" state, as if the image was fitted to the container.
        const fitScale = capFitScale(Math.min(container.width / imgSize.width, container.height / imgSize.height));
        const initialPos = {
          x: (container.width - imgSize.width) / 2,
          y: (container.height - imgSize.height) / 2,
        };

        // Now, use the logic from zoomImage to transition from the "fit" state to the 100% state.
        const newScale = getActualSizeScale();
        const imageOffsetX = ((fitScale - newScale) * ((cursorX - initialPos.x) - imgSize.width / 2)) / fitScale;
        const imageOffsetY = ((fitScale - newScale) * ((cursorY - initialPos.y) - imgSize.height / 2)) / fitScale;
        
        scale.value[imgIndex] = newScale;
        position.value[imgIndex] = {
          x: initialPos.x + imageOffsetX,
          y: initialPos.y + imageOffsetY,
        };
        triggerRef(position);
        clampPosition(true);
      }

      // Also update the other image's position to match for smooth transitions
      // const otherImageIndex = activeImage.value ^ 1;
      // imageSrc.value[otherImageIndex] = '';
      // position.value[otherImageIndex] = position.value[activeImage.value];
    } else {
      // For isZoomFit, the original logic is fine.
      updateZoomFit(true);
    }

    setTimeout(() => {
      noTransition.value = false;
    }, 500);
  };

  if (containerSize.value.width > 0) {
    applyZoom();
  } else {
    const unwatch = watch(containerSize, (newSize) => {
      if (newSize.width > 0) {
        applyZoom();
        unwatch();
      }
    });
  }
};

const rotateView = (delta = 90) => {
  imageRotate.value[activeImage.value] += delta;
};

// ---- Typed zoom: type digits (e.g. 5 5) to set that zoom in percent ---------
let zoomDigits = '';
let zoomDigitsTimer: number | null = null;
const ZOOM_DIGITS_DELAY_MS = 700;

function setZoomPercent(percent: number): boolean {
  const imgIndex = activeImage.value;
  const imgSize = imageSize.value[imgIndex];
  const container = containerSize.value;
  if (!(imgSize.width > 0) || !(container.width > 0) || !(percent > 0)) return false;
  const dpr = window.devicePixelRatio || 1;
  const target = Math.min(Math.max(percent / 100 / dpr, minScale.value), maxScale.value);
  zoomImage(container.width / 2, container.height / 2, target);
  toast.info(t('image_viewer.zoom_set', { zoom: Math.round(target * dpr * 100) }));
  return true;
}

function commitZoomDigits() {
  zoomDigitsTimer = null;
  const percent = Number(zoomDigits);
  zoomDigits = '';
  if (percent > 0) setZoomPercent(percent);
}

function typeZoomDigit(digit: string): boolean {
  if (!/^[0-9]$/.test(digit) || props.isSlideShow) return false;
  zoomDigits = (zoomDigits + digit).slice(-4);
  if (zoomDigitsTimer !== null) clearTimeout(zoomDigitsTimer);
  zoomDigitsTimer = window.setTimeout(commitZoomDigits, ZOOM_DIGITS_DELAY_MS);
  return true;
}

onBeforeUnmount(() => {
  if (zoomDigitsTimer !== null) clearTimeout(zoomDigitsTimer);
});

function toggleZoomLock(): boolean {
  zoomLocked.value = !zoomLocked.value;
  toast.info(t(zoomLocked.value ? 'image_viewer.zoom_lock_on' : 'image_viewer.zoom_lock_off'));
  return zoomLocked.value;
}

const toggleZoomFit = () => {
  if (props.isSlideShow) return;
  // Zoomed in beyond fit: a double click always goes back to fit-to-screen.
  const fit = getFitScale();
  if (fit > 0 && scale.value[activeImage.value] > fit * 1.01) {
    zoomFit();
    if (!props.isZoomFit) emit('update:isZoomFit', true);
    return;
  }
  emit('update:isZoomFit', !props.isZoomFit);
};

// Explicit "fit to screen" (not a toggle).
const zoomFitScreen = () => {
  if (props.isSlideShow) return;
  zoomFit();
  if (!props.isZoomFit) emit('update:isZoomFit', true);
};

// Explicit 1:1 (actual pixels), at the mouse position like the click-and-hold zoom.
const zoomActualSize = () => {
  if (props.isSlideShow) return;
  if (props.isZoomFit) {
    emit('update:isZoomFit', false);   // the zoom-fit watcher then resets to 1:1
  } else {
    zoomReset();
  }
};

const updateZoomFit = (force: boolean = false) => {
  console.log('updateZoomFit');
  isZoomFit.value ? zoomFit(force) : zoomReset(force);

  // set the hide image to the same position
  // const nextImageIndex = activeImage.value ^ 1;
  // imageSrc.value[nextImageIndex] = '';
  // position.value[nextImageIndex] = position.value[activeImage.value];
};

// "Don't enlarge small photos": fit-to-screen never goes above 100 % (actual pixels).
function capFitScale(fit: number): number {
  return config.settings.fitNoUpscale === true ? Math.min(fit, getActualSizeScale()) : fit;
}

watch(() => config.settings.fitNoUpscale, () => {
  if (isZoomFit.value && containerSize.value.width > 0) zoomFit(true);
});

// Zoom to fit image in container
const zoomFit = (force: boolean = false) => {
  console.log('zoomFit');
  const container = containerSize.value;
  const imgRotatedSize = imageSizeRotated.value[activeImage.value];
  
  const containerAspectRatio = container.width / container.height;
  const imageAspectRatio = imgRotatedSize.width / imgRotatedSize.height;

  const scale = capFitScale(containerAspectRatio > imageAspectRatio 
    ? container.height / imgRotatedSize.height
    : container.width / imgRotatedSize.width);

  // set position to center
  zoomImage(container.width / 2, container.height / 2, scale, force);
};

// Reset zoom level and position
const zoomReset = (force: boolean = false) => {
  console.log('zoomReset');
  updatePosition();
  const mousePos = mousePosition.value;
  const containerPosVal = containerPos.value;
  zoomImage(mousePos.x - containerPosVal.x, mousePos.y - containerPosVal.y, getActualSizeScale(), force);
};

let dragOutStart: { x: number; y: number; path: string; preview: number[] } | null = null;
function trackImageDragOut(event: MouseEvent) {
  const start = dragOutStart;
  if (!start || !(event.buttons & 1)) return;
  if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 6 || !isWindowDragEdge(event)) return;
  mouseDragNavDeltaX.value = 0;
  mouseDragNavDeltaY.value = 0;
  finishImageMouseDrag(false);
  void startNativeFileDrag([start.path], start.preview).catch(error => {
    console.error('Native image drag failed:', error);
    toast.error(t('tooltip.drag_out.failed'));
  });
}

// ---- Press-and-hold "loupe": hold the left button to zoom to 100 % under the
// cursor (drag to pan); release to return to the previous zoom and position.
const LOUPE_HOLD_MS = 280;
const LOUPE_MOVE_TOLERANCE = 6;
let loupeTimer: ReturnType<typeof setTimeout> | null = null;
let loupeStart = { x: 0, y: 0 };
let loupeSaved: { index: number; scale: number; x: number; y: number } | null = null;

// The loupe jump (and the return) animate like step zooming, even while the button is held.
const loupeAnimating = ref(false);
let loupeAnimTimer: ReturnType<typeof setTimeout> | null = null;
function startLoupeAnimation() {
  loupeAnimating.value = true;
  if (loupeAnimTimer) clearTimeout(loupeAnimTimer);
  loupeAnimTimer = setTimeout(() => { loupeAnimating.value = false; loupeAnimTimer = null; }, 190);
}

function cancelLoupeTimer() {
  if (loupeTimer) clearTimeout(loupeTimer);
  loupeTimer = null;
}

function startLoupeTimer(event: MouseEvent) {
  cancelLoupeTimer();
  loupeStart = { x: event.clientX, y: event.clientY };
  if (props.isSlideShow || loadError.value || isLoading.value) return;
  prefetchActiveHybridFull();
  prefetchTileAtCursor(event);
  loupeTimer = setTimeout(() => {
    loupeTimer = null;
    beginLoupe();
  }, LOUPE_HOLD_MS);
}

function beginLoupe() {
  if (!isDraggingImage.value || loupeSaved) return;
  const index = activeImage.value;
  const target = getActualSizeScale();
  const current = scale.value[index];
  if (!(imageSizeRotated.value[index].width > 0) || Math.abs(current - target) / target < 0.01) return;

  updatePosition();
  loupeSaved = { index, scale: current, x: position.value[index].x, y: position.value[index].y };
  startLoupeAnimation();
  zoomImage(loupeStart.x - containerPos.value.x, loupeStart.y - containerPos.value.y, target, true);
  window.addEventListener('mouseup', endLoupe, { once: true, capture: true });
}

function endLoupe() {
  cancelLoupeTimer();
  window.removeEventListener('mouseup', endLoupe, true);
  const saved = loupeSaved;
  loupeSaved = null;
  if (!saved || activeImage.value !== saved.index) return;
  startLoupeAnimation();
  scale.value[saved.index] = saved.scale;
  position.value[saved.index] = { x: saved.x, y: saved.y };
  clampPosition(true);
}

function releaseNoTransition() {
  requestAnimationFrame(() => requestAnimationFrame(() => { noTransition.value = false; }));
}

// start dragging
const handleImageMouseDown = (event: MouseEvent) => {
  if (isTouchActive.value) return; // touch path owns this gesture
  event.preventDefault();
  updatePosition();

  if (event.button === 0) {     // left click: drag image
    if (!props.originalUnavailable && !props.isSlideShow && props.filePath) {
      dragOutStart = {
        x: event.clientX, y: event.clientY, path: props.filePath,
        preview: createDragPreview(event.target as HTMLImageElement),
      };
      document.addEventListener('mousemove', trackImageDragOut, true);
      document.documentElement.addEventListener('mouseleave', trackImageDragOut);
      document.addEventListener('mouseup', handleImageMouseUp, true);
    }
    isDraggingImage.value = true;
    startLoupeTimer(event);
    lastMousePosition.value = { x: event.clientX, y: event.clientY };
    mouseDragNavDeltaX.value = 0;
    mouseDragNavDeltaY.value = 0;
    mouseDragNavTriggered.value = false;
  } else if (event.button === 2) { // right click: toggle zoom fit
    // TODO: use context menu
    // isZoomFit.value = !isZoomFit.value;
    // updateZoomFit();
  } else if (event.button === 1) { // middle button
    // emit('message-from-image', { message: 'showInfoPanel' });
  } else if (event.button === 3) {  // back button
    emit('message-from-image-viewer', { message: 'prev' });
  } else if (event.button === 4) {  // forward button
    emit('message-from-image-viewer', { message: 'next' });
  } 
};

const handleImageMouseMove = (event: MouseEvent) => {
  if (isTouchActive.value) return; // touch path owns this gesture
  // update mouse position
  mousePosition.value = { x: event.clientX, y: event.clientY };
  updatePosition();
  onTileHover(event);

  if (!isDraggingImage.value) return;

  if (loupeTimer && Math.hypot(event.clientX - loupeStart.x, event.clientY - loupeStart.y) > LOUPE_MOVE_TOLERANCE) {
    cancelLoupeTimer();
  }
  latestMouseEvent.value = event;

  if (animationFrameId) {
    cancelAnimationFrame(animationFrameId);
  }

  animationFrameId = requestAnimationFrame(updateDragPosition);
};

// stop dragging
const handleImageMouseUp = () => finishImageMouseDrag(true);

function finishImageMouseDrag(navigate: boolean) {
  if (animationFrameId) cancelAnimationFrame(animationFrameId);
  animationFrameId = null;
  // A quick swipe may release before its scheduled frame runs.
  if (navigate && isDraggingImage.value && latestMouseEvent.value) updateDragPosition();
  latestMouseEvent.value = null;
  endLoupe(); // after the final pan flush, so the restored view is not nudged
  if (navigate && mouseDragNavTriggered.value && Math.abs(mouseDragNavDeltaX.value) >= MOUSE_DRAG_NAV_THRESHOLD) {
    const direction = mouseDragNavDeltaX.value > 0 ? 'prev' : 'next';
    navDirection.value = direction;
    emit('message-from-image-viewer', { message: direction });
  }
  dragOutStart = null;
  document.removeEventListener('mousemove', trackImageDragOut, true);
  document.documentElement.removeEventListener('mouseleave', trackImageDragOut);
  document.removeEventListener('mouseup', handleImageMouseUp, true);
  isDraggingImage.value = false;
  mouseDragNavDeltaX.value = 0;
  mouseDragNavDeltaY.value = 0;
  mouseDragNavTriggered.value = false;
}

// mouse leave
// reset mouse position to the center when leaving the container
const handleImageMouseLeave = () => {
  clearTileHover();
  // purpose: when clicking zoom fit/reset, the image will be centered
  // and the mouse position will be set to the center of the container
  const container = containerSize.value;
  mousePosition.value = { x: container.width / 2, y: container.height / 2 };
};

const updateDragPosition = () => {
  const event = latestMouseEvent.value;
  if (!event || !isDraggingImage.value) return;

  const imgIndex = activeImage.value;
  const scaleVal = scale.value[imgIndex];
  const imageRotatedSize = imageSizeRotated.value[imgIndex];
  const container = containerSize.value;
  const lastPos = lastMousePosition.value;

  const scaledWidth = imageRotatedSize.width * scaleVal;
  const scaledHeight = imageRotatedSize.height * scaleVal;
  const rawDeltaX = event.clientX - lastPos.x;
  const rawDeltaY = event.clientY - lastPos.y;
  const canPan = scaledWidth > container.width || scaledHeight > container.height;

  // In zoom-fit mode, horizontal drag acts like touchpad swipe navigation
  // only when the image cannot be panned in current viewport.
  if (isZoomFit.value && !props.isSlideShow && !canPan && !loupeSaved) {
    mouseDragNavDeltaX.value += rawDeltaX;
    mouseDragNavDeltaY.value += rawDeltaY;

    // Pen/mouse drag on a fit-to-window image: jump to 100% under the press
    // point and keep dragging to pan (released -> back to fit). Dragging an
    // already zoomed image pans it as usual.
    if (config.settings.fitDragLoupe !== false
      && Math.hypot(mouseDragNavDeltaX.value, mouseDragNavDeltaY.value) >= LOUPE_MOVE_TOLERANCE) {
      cancelLoupeTimer();
      beginLoupe();
      if (loupeSaved) {
        mouseDragNavDeltaX.value = 0;
        mouseDragNavDeltaY.value = 0;
        lastMousePosition.value = { x: event.clientX, y: event.clientY };
        animationFrameId = null;
        return;
      }
    }

    if (!mouseDragNavTriggered.value) {
      const absX = Math.abs(mouseDragNavDeltaX.value);
      const absY = Math.abs(mouseDragNavDeltaY.value);
      if (absX >= MOUSE_DRAG_NAV_THRESHOLD && absX > absY) {
        // Commit mouse swipe on release, so dragging onward to the window
        // edge can hand the original file to the OS instead of changing images.
        mouseDragNavTriggered.value = true;
      }
    }

    lastMousePosition.value = { x: event.clientX, y: event.clientY };
    animationFrameId = null;
    return;
  }

  const deltaX = scaledWidth <= container.width ? 0 : event.clientX - lastPos.x;
  const deltaY = scaledHeight <= container.height ? 0 : event.clientY - lastPos.y;

  position.value[imgIndex].x += deltaX;
  position.value[imgIndex].y += deltaY;

  lastMousePosition.value = { x: event.clientX, y: event.clientY };

  clampPosition();

  animationFrameId = null; // reset animation frame ID
};

// Simple reset - clear all swipe state
function resetSwipeState() {
  gestureType.value = 'none';
  horizontalDeltaAccumulator = 0;
  verticalDeltaAccumulator = 0;
  hasNavigatedThisGesture = false;
  lastDeltaX = 0;
  navDirection.value = '';
  noTransition.value = false;
  if (gestureResetTimeout) {
    clearTimeout(gestureResetTimeout);
    gestureResetTimeout = null;
  }
}

function handleTransitionEnd() {
  navDirection.value = '';
}

// mouse wheel zoom
function handleImageWheel(event: WheelEvent) {
  event.preventDefault();
  event.stopPropagation();
  updatePosition();

  // Touchpad pinch (and Ctrl+wheel) arrives as a wheel event with ctrlKey=true.
  // Treat it as a direct zoom, bypassing the swipe/nav gesture-detection path.
  if (event.ctrlKey) {
    isWheelZooming.value = true;
    if (wheelZoomTimeout) clearTimeout(wheelZoomTimeout);
    wheelZoomTimeout = setTimeout(() => {
      isWheelZooming.value = false;
    }, 200);
    applyZoomFromWheel(event);
    return;
  }

  // Simple touchpad detection: if there's horizontal delta, it's a touchpad
  // Mouse wheels only scroll vertically (deltaY only)
  // Once detected, stays true for the session (sticky)
  if (event.deltaX !== 0) {
    isTouchpadDevice = true;
  }
  
  const isTouchPad = isTouchpadDevice;

  // Reset timeout - when no events for 150ms, reset gesture state
  if (gestureResetTimeout) clearTimeout(gestureResetTimeout);
  gestureResetTimeout = setTimeout(() => {
    resetSwipeState();
  }, 150);

  if (isTouchPad) {
    // If already navigated this gesture, check if speed increased
    if (hasNavigatedThisGesture) {
      const speedIncreased = Math.abs(event.deltaX) > Math.abs(lastDeltaX) + 5;
      if (!speedIncreased) {
        lastDeltaX = event.deltaX;
        return; // Block - not a new intentional flick
      }
      // Speed increased - allow new navigation
      hasNavigatedThisGesture = false;
      horizontalDeltaAccumulator = 0;
    }
    lastDeltaX = event.deltaX;

    // Determine gesture direction
    if (gestureType.value === 'none') {
      horizontalDeltaAccumulator += event.deltaX;
      verticalDeltaAccumulator += event.deltaY;

      const absX = Math.abs(horizontalDeltaAccumulator);
      const absY = Math.abs(verticalDeltaAccumulator);

      if (absX > GESTURE_LOCK_THRESHOLD || absY > GESTURE_LOCK_THRESHOLD) {
        gestureType.value = absX > absY ? 'nav' : 'zoom';
      }
      return;
    }

    if (gestureType.value === 'nav') {
      horizontalDeltaAccumulator += event.deltaX;

      // Trigger navigation when threshold reached
      if (Math.abs(horizontalDeltaAccumulator) >= HORIZONTAL_NAV_THRESHOLD) {
        const direction = horizontalDeltaAccumulator > 0 ? 'next' : 'prev';
        navDirection.value = direction;
        hasNavigatedThisGesture = true;
        horizontalDeltaAccumulator = 0;
        gestureType.value = 'none'; // Allow new image to be centered
        emit('message-from-image-viewer', { message: direction });
      }
      return;
    }
  }

  // If we're here and it's a touchpad, gestureType must be 'zoom' or it's a regular mouse
  isWheelZooming.value = true;
  if (wheelZoomTimeout) clearTimeout(wheelZoomTimeout);
  wheelZoomTimeout = setTimeout(() => {
    isWheelZooming.value = false;
  }, 200);

  const zoomFactor = isTouchPad ? 1 : 0.1; // Adjust sensitivity

  // Touchpad always zooms; mouseWheelMode only affects regular mouse
  if (isTouchPad) {
    wheelZoom(event, zoomFactor);
  } else if (config.settings.mouseWheelMode === 0) {  // 0: previous/next image
    if (event.ctrlKey) {     // ctrl + mouse wheel: zoom in / out
      wheelZoom(event, zoomFactor);
    } else {
      emit('message-from-image-viewer', { message: event.deltaY < 0 ? 'prev' : 'next' });
    }
  } else if (config.settings.mouseWheelMode === 1) {  // 1: zoom in / out
    wheelZoom(event, zoomFactor);
  }
}

// wheel zoom - Industry standard fixed-step approach
function wheelZoom(event: WheelEvent, zoomFactor: number) {
  const currentScale = scale.value[activeImage.value];
  
  // Normalize delta to wheel "notches" (standard mouse ~100 per notch)
  let delta = event.deltaY;
  if (event.deltaMode === 1) { // DOM_DELTA_LINE
    delta *= 40;
  } else if (event.deltaMode === 2) { // DOM_DELTA_PAGE
    delta *= 800;
  }
  
  // Convert to notches (standard mice report ~100 per notch)
  // Use sign for direction, clamp magnitude for consistency
  const notches = Math.sign(delta) * (Math.abs(delta) / 100);
  
  // Fixed 20% zoom per notch
  const ZOOM_FACTOR = 0.2;
  const multiplier = Math.pow(1 + ZOOM_FACTOR, -notches);
  
  let newScale = currentScale * multiplier;
  newScale = Math.min(Math.max(newScale, minScale.value), maxScale.value);

  // Zoom at cursor position in container layout coordinates.
  if (container.value) {
    const rect = (container.value as HTMLElement).getBoundingClientRect();
    const containerSizeVal = containerSize.value;
    const x = ((event.clientX - rect.left) * containerSizeVal.width) / rect.width;
    const y = ((event.clientY - rect.top) * containerSizeVal.height) / rect.height;
    zoomImage(x, y, newScale);
    return;
  }

  // Fallback for rare lifecycle gaps.
  const containerPosVal = containerPos.value;
  zoomImage(event.clientX - containerPosVal.x, event.clientY - containerPosVal.y, newScale);
}

function getFitWidthScale(): number {
  const container = containerSize.value;
  const rotated = imageSizeRotated.value[activeImage.value];
  if (!(container.width > 0) || !(rotated.width > 0)) return 0;
  return container.width / rotated.width;
}

// Discrete zoom levels (in real screen pixels per image pixel) used by the
// zoom in/out commands: 10, 20, 30, 50, 70, 100, 200 %, then larger steps.
const ZOOM_STEPS = [0.1, 0.2, 0.3, 0.5, 0.7, 1, 2, 3, 4, 5, 7, 10];

// The steps plus the "fit to width" zoom of the current image (when it is a sensible step).
function getZoomSteps(dpr: number): number[] {
  const steps = [...ZOOM_STEPS];
  if (typeof getFitWidthScale === 'function') {
    const fitWidth = getFitWidthScale() * dpr;
    if (fitWidth > 0.1 && fitWidth < 10 && !steps.some(step => Math.abs(step / fitWidth - 1) < 0.01)) {
      steps.push(fitWidth);
      steps.sort((a, b) => a - b);
    }
  }
  return steps;
}

const zoomIn = () => {
  const dpr = window.devicePixelRatio || 1;
  const current = scale.value[activeImage.value] * dpr;
  const next = getZoomSteps(dpr).find(step => step > current * 1.005);
  const newScale = Math.min(next !== undefined ? next / dpr : scale.value[activeImage.value], maxScale.value);
  const container = containerSize.value;
  zoomImage(container.width / 2, container.height / 2, newScale);
};

const zoomOut = () => {
  const container = containerSize.value;
  minScale.value = 0.1;
  const dpr = window.devicePixelRatio || 1;
  const current = scale.value[activeImage.value] * dpr;
  const previous = getZoomSteps(dpr).reverse().find(step => step < current * 0.995);
  const newScale = Math.max(previous !== undefined ? previous / dpr : minScale.value, minScale.value);
  zoomImage(container.width / 2, container.height / 2, newScale);
};

const zoomFitWidth = () => {
  const target = getFitWidthScale();
  if (!(target > 0)) return;
  const container = containerSize.value;
  // Fit to width: show the top of the photo at the new zoom.
  zoomImage(container.width / 2, 0, Math.min(Math.max(target, 0.01), maxScale.value));
};

const zoomActual = () => {
  const container = containerSize.value;
  zoomImage(container.width / 2, container.height / 2, getActualSizeScale());
};

// Zoom image at cursor position
function zoomImage(cursorX: number, cursorY: number, newScale: number, force: boolean = false) {
  const imgIndex = activeImage.value;
  const currentScale = scale.value[imgIndex];
  const pos = position.value[imgIndex];
  const imgSize = imageSize.value[imgIndex];
  
  const imageOffsetX = ((currentScale - newScale) * ((cursorX - pos.x) - imgSize.width / 2)) / currentScale;
  const imageOffsetY = ((currentScale - newScale) * ((cursorY - pos.y) - imgSize.height / 2)) / currentScale;
  
  pos.x += imageOffsetX;
  pos.y += imageOffsetY;

  scale.value[imgIndex] = newScale;
  clampPosition(force);
}

// ---- Saved viewport (per-file zoom + position) ----------------------------
function setHasSavedViewport(value: boolean) {
  hasSavedViewport.value = value;
  // Always emit: a freshly mounted Image must resync the parent's flag.
  emit('saved-viewport-change', value);
}

function restoreSavedViewport(saved: SavedViewport): boolean {
  const imgIndex = activeImage.value;
  const imgSize = imageSize.value[imgIndex];
  if (!(imgSize.width > 0) || !(containerSize.value.width > 0)) return false;

  const dpr = window.devicePixelRatio || 1;
  // Keep the same real zoom even if the layout width differs from the save time.
  const widthRatio = saved.sourceWidth > 0 ? saved.sourceWidth / imgSize.width : 1;
  const scaleValue = (saved.displayScale * widthRatio) / dpr;

  if (zoomFitBeforeRestore === null) zoomFitBeforeRestore = props.isZoomFit;
  isZoomFit.value = false;
  if (props.isZoomFit) {
    skipNextZoomFitWatch = true;
    emit('update:isZoomFit', false);
  }

  applyViewportState({ scale: scaleValue, normX: saved.normX, normY: saved.normY }, true);
  emit('viewport-change', getViewportState());
  return true;
}

// Is a zoom (physical px per image px) at or below this image's fit-to-screen zoom?
function isScaleAtOrBelowFit(displayScale: number, sourceWidth: number): boolean {
  const fit = getFitScale();
  if (!(fit > 0)) return false;
  const imgSize = imageSize.value[activeImage.value];
  const widthRatio = sourceWidth > 0 && imgSize.width > 0 ? sourceWidth / imgSize.width : 1;
  return (displayScale * widthRatio) / (window.devicePixelRatio || 1) <= fit * 1.005;
}

// Follow mode: the next image takes over the shared viewport (and keeps it saved), unless the
// zoom would not exceed its fit-to-screen size; then it is shown fitted.
function applyFollowedViewport(): boolean {
  const shared = followViewport.value;
  const path = imageFilePath.value[activeImage.value];
  const imgSize = imageSize.value[activeImage.value];
  if (!shared || !path || !(imgSize.width > 0) || !(containerSize.value.width > 0)) return false;

  if (isScaleAtOrBelowFit(shared.displayScale, shared.sourceWidth)) {
    isZoomFit.value = true;
    if (!props.isZoomFit) emit('update:isZoomFit', true);
    zoomFit(true);
    return true;
  }
  if (!restoreSavedViewport({ ...shared, savedAt: Date.now() })) return false;
  persistCurrentViewport(path);
  setHasSavedViewport(true);
  return true;
}

function persistCurrentViewport(path: string) {
  const imgSize = imageSize.value[activeImage.value];
  const viewport = getViewportState();
  const entry = {
    displayScale: viewport.scale * (window.devicePixelRatio || 1),
    normX: viewport.normX,
    normY: viewport.normY,
    sourceWidth: imgSize.width,
  };
  setSavedViewport(path, entry);
  if (props.allowViewportFollow && followViewport.value) followViewport.value = entry;
}

// A change of zoom or position on an image with a saved viewport (or in follow mode) is saved
// and, in follow mode, becomes the viewport of the images opened next.
let viewportPersistTimer: ReturnType<typeof setTimeout> | null = null;
function schedulePersistViewport() {
  if (!hasSavedViewport.value && !(props.allowViewportFollow && followViewport.value)) return;
  if (noTransition.value || loupeSaved || isLoading.value) return;
  if (viewportPersistTimer) clearTimeout(viewportPersistTimer);
  viewportPersistTimer = setTimeout(() => {
    viewportPersistTimer = null;
    const path = imageFilePath.value[activeImage.value];
    if (!path || path !== props.filePath || noTransition.value || loupeSaved) return;
    if (!(imageSize.value[activeImage.value].width > 0)) return;
    if (isAtFitScale()) {
      // Back at fit: the image no longer pins a viewport (the followed one stays as it is).
      if (hasSavedViewport.value && removeSavedViewport(path)) setHasSavedViewport(false);
      return;
    }
    persistCurrentViewport(path);
    setHasSavedViewport(true);
  }, 400);
}
watch(() => {
  const i = activeImage.value;
  return [scale.value[i], position.value[i]?.x, position.value[i]?.y];
}, schedulePersistViewport);

function getFitScale(): number {
  const container = containerSize.value;
  const rotated = imageSizeRotated.value[activeImage.value];
  if (!(container.width > 0) || !(container.height > 0) || !(rotated.width > 0) || !(rotated.height > 0)) return 0;
  return capFitScale(Math.min(container.width / rotated.width, container.height / rotated.height));
}

function isAtFitScale(): boolean {
  const fit = getFitScale();
  if (!(fit > 0)) return true;
  return Math.abs(scale.value[activeImage.value] / fit - 1) < 0.005;
}

function saveViewport(): boolean {
  const path = imageFilePath.value[activeImage.value];
  const imgSize = imageSize.value[activeImage.value];
  if (!path || path !== props.filePath || !(imgSize.width > 0)) return false;
  // isZoomFit is only the fit/actual *mode*; wheel and step zooming leave it
  // untouched. Compare the real scale with the fit scale instead.
  if (isAtFitScale()) {
    toast.info(t('image_viewer.viewport_fit_hint'));
    return false;
  }
  const viewport = getViewportState();
  const displayScale = viewport.scale * (window.devicePixelRatio || 1);
  setSavedViewport(path, {
    displayScale,
    normX: viewport.normX,
    normY: viewport.normY,
    sourceWidth: imgSize.width,
  });
  // The displayed image is now "pinned"; keep the mode that preceded it.
  setHasSavedViewport(true);
  // Image Viewer window: carry this viewport to every image opened next.
  if (props.allowViewportFollow) {
    followViewport.value = { displayScale, normX: viewport.normX, normY: viewport.normY, sourceWidth: imgSize.width };
  }
  toast.success(t('image_viewer.viewport_saved', { zoom: Math.round(displayScale * 100) }));
  return true;
}

function clearSavedViewport(): boolean {
  const path = imageFilePath.value[activeImage.value];
  if (followViewport.value) {
    followViewport.value = null;
    if (path && removeSavedViewport(path)) setHasSavedViewport(false);
    toast.success(t('image_viewer.viewport_cleared'));
    return true;
  }
  if (!path || !removeSavedViewport(path)) {
    if (path) toast.info(t('image_viewer.viewport_none'));
    return false;
  }
  setHasSavedViewport(false);
  toast.success(t('image_viewer.viewport_cleared'));
  return true;
}

function toggleSavedViewport() {
  if (hasSavedViewport.value) clearSavedViewport();
  else saveViewport();
}

function getViewportState() {
  const imgIndex = activeImage.value;
  const imgSize = imageSize.value[imgIndex];
  const scaleVal = scale.value[imgIndex];
  const container = containerSize.value;
  const pos = position.value[imgIndex];

  // Store the image-content point under the viewport center, not the rendered
  // x/y offset. That makes the viewport independent from source dimensions.
  const dx = container.width / 2 - (pos.x + imgSize.width / 2);
  const dy = container.height / 2 - (pos.y + imgSize.height / 2);
  const angle = imageRotate.value[imgIndex] * Math.PI / 180;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const contentX = imgSize.width / 2 + (dx * cos + dy * sin) / scaleVal;
  const contentY = imgSize.height / 2 + (-dx * sin + dy * cos) / scaleVal;

  return {
    scale: scaleVal,
    normX: Math.min(Math.max(contentX / imgSize.width, 0), 1),
    normY: Math.min(Math.max(contentY / imgSize.height, 0), 1),
    sourceWidth: imgSize.width,
    sourceHeight: imgSize.height,
    fileId: props.fileId,
    viewportWidth: container.width,
    viewportHeight: container.height,
    rotate: imageRotate.value[imgIndex],
    pannable: isGrabbing.value,
  };
}

function applyViewportState(viewport: { scale?: number; normX?: number; normY?: number }, silent = false) {
  if (!viewport || typeof viewport.scale !== 'number') return;

  const imgIndex = activeImage.value;
  const imgSize = imageSize.value[imgIndex];
  const container = containerSize.value;
  const safeScale = Math.min(Math.max(viewport.scale, minScale.value), maxScale.value);

  scale.value[imgIndex] = safeScale;

  const normX = Math.min(Math.max(viewport.normX ?? 0.5, 0), 1);
  const normY = Math.min(Math.max(viewport.normY ?? 0.5, 0), 1);
  const localX = (normX - 0.5) * imgSize.width * safeScale;
  const localY = (normY - 0.5) * imgSize.height * safeScale;
  const angle = imageRotate.value[imgIndex] * Math.PI / 180;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const rotatedX = localX * cos - localY * sin;
  const rotatedY = localX * sin + localY * cos;

  position.value[imgIndex].x = container.width / 2 - imgSize.width / 2 - rotatedX;
  position.value[imgIndex].y = container.height / 2 - imgSize.height / 2 - rotatedY;

  if (silent) {
    // Sync path: disable transition for this frame to avoid trailing.
    noTransition.value = true;
  }

  suppressViewportEmit.value = silent;
  clampPosition(true);
  suppressViewportEmit.value = false;

  if (silent) {
    requestAnimationFrame(() => {
      noTransition.value = false;
    });
  }
}

// Ensure image stays within container
function clampPosition(force: boolean = false) {
  // Skip clamping during horizontal swipe to avoid jitter
  if (!force && gestureType.value === 'nav') return;
  
  const imgIndex = activeImage.value;
  const imgRotatedSize = imageSizeRotated.value[imgIndex];
  const imgSize = imageSize.value[imgIndex];
  const scaleVal = scale.value[imgIndex];
  const container = containerSize.value;
  const pos = position.value[imgIndex];

  const paddingX = (imgRotatedSize.width * scaleVal - imgSize.width) / 2;
  const paddingY = (imgRotatedSize.height * scaleVal - imgSize.height) / 2;
  const maxX = container.width - imgRotatedSize.width * scaleVal + paddingX;
  const maxY = container.height - imgRotatedSize.height * scaleVal + paddingY;

  isGrabbing.value = false;
  // Optional: let the photo be dragged past the screen edges, keeping a strip of it visible.
  const minVisible = 100;
  const overX = config.settings.panBeyondEdges ? Math.max(0, container.width - minVisible) : 0;
  const overY = config.settings.panBeyondEdges ? Math.max(0, container.height - minVisible) : 0;
  if (Math.floor(imgRotatedSize.width * scaleVal) > container.width) {
    pos.x = Math.min(Math.max(pos.x, maxX - overX), paddingX + overX);
    isGrabbing.value = true;
  } else {
    pos.x = (container.width - imgSize.width) / 2;
  }
  if (Math.floor(imgRotatedSize.height * scaleVal) > container.height) {
    pos.y = Math.min(Math.max(pos.y, maxY - overY), paddingY + overY);
    isGrabbing.value = true;
  } else {
    pos.y = (container.height - imgSize.height) / 2;
  }
  triggerRef(position);

  if (!suppressViewportEmit.value) {
    emit('viewport-change', getViewportState());
  }
};

// Expose methods
defineExpose({
  cycleRawPreview,
  zoomIn, 
  zoomOut,
  zoomActual,
  zoomFitWidth,
  zoomFitScreen,
  zoomActualSize,
  rotateView,
  getViewportState,
  applyViewportState,
  saveViewport,
  clearSavedViewport,
  toggleSavedViewport,
  setZoomPercent,
  typeZoomDigit,
  toggleZoomLock,
  zoomNavigator,
  navigateImage,
  toggleZoomFit,
  getCurrentImageSrc: () => imageSrc.value[activeImage.value] || '',
  clearPreloadCache: (filePath?: string) => {
    if (filePath) {
      preloadCache.delete(filePath);
    } else {
      preloadCache.clear();
    }
  },
});

</script>

<style scoped>
/* Slideshow / Swipe transition */
.slide-next-enter-active,
.slide-next-leave-active,
.slide-prev-enter-active,
.slide-prev-leave-active {
  transition: transform 0.6s cubic-bezier(0.4, 0, 0.2, 1);
  will-change: transform;
  backface-visibility: hidden;
  transform: translateZ(0);
  contain: paint;
}

/* next: current leaves left, new enters from right */
.slide-next-enter-from {
  transform: translate3d(100%, 0, 0);
}
.slide-next-leave-to {
  transform: translate3d(-100%, 0, 0);
}

/* prev: current leaves right, new enters from left */
.slide-prev-enter-from {
  transform: translate3d(-100%, 0, 0);
}
.slide-prev-leave-to {
  transform: translate3d(100%, 0, 0);
}

.slide-next-enter-active,
.slide-prev-enter-active {
  z-index: 2;
}

.slide-next-leave-active,
.slide-prev-leave-active {
  z-index: 1;
}

.slideshow-fade-enter-active,
.slideshow-fade-leave-active {
  transition:
    opacity 0.5s cubic-bezier(0.22, 1, 0.36, 1),
    filter 0.5s ease;
  will-change: opacity, filter;
}

.slideshow-fade-enter-active {
  z-index: 2;
}

.slideshow-fade-leave-active {
  z-index: 1;
}

.slideshow-fade-enter-from {
  opacity: 0;
  filter: brightness(0.88);
}

.slideshow-fade-enter-to {
  opacity: 1;
  filter: brightness(1);
}

.slideshow-fade-leave-from {
  opacity: 1;
  filter: brightness(1);
}

.slideshow-fade-leave-to {
  opacity: 0;
  filter: brightness(0.72);
}

</style>
