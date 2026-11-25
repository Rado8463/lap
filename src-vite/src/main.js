import { createApp, watch } from 'vue'
import { i18n, initializeLanguage, resolveSystemLanguage } from '@/common/i18n'
import { locale as getSystemLanguage } from '@tauri-apps/plugin-os'
import { createPinia } from 'pinia'
import piniaPersistedState from 'pinia-plugin-persistedstate'
import { emit, listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { message } from '@tauri-apps/plugin-dialog'
import { initializeOriginalMedia } from '@/common/utils'
import { standaloneViewer } from '@/common/standalone'
import { setShortcutOverrides } from '@/common/shortcuts'
import { getSystemRegionLocale } from '@/common/api'
import 'cally'
import router from '@/common/router'
import App from '@/App.vue'
import { normalizeStatusBarFields } from '@/common/statusBarFields'
import { useConfigStore } from '@/stores/configStore'
import '@/assets/app.css'

performance.mark('startup-frontend-started')

// Create the app instance
const app = createApp(App)

// Create Pinia store and use the persisted state plugin
const pinia = createPinia()
pinia.use(piniaPersistedState)
app.use(pinia) // Use Pinia
const config = useConfigStore() // Use the config store
const currentWindowLabel = getCurrentWebviewWindow().label
// Lap started with an image path (Explorer "Open with"): the 'main' window only hosts the viewer.
standaloneViewer.value = currentWindowLabel === 'main' && !!(await invoke('get_standalone_launch').catch(() => null))
const isMainWindow = currentWindowLabel === 'main' && !standaloneViewer.value
const isSettingsWindow = currentWindowLabel === 'settings'
let settingsSyncRequest = 0
const originalMediaReady = initializeOriginalMedia().then(() => true, error => {
  console.error(error)
  return false
})

listen('status-bar-fields-changed', ({ payload }) => {
  const fields = normalizeStatusBarFields(payload)
  if (JSON.stringify(config.settings.statusBarFields) !== JSON.stringify(fields)) {
    config.settings.statusBarFields = fields
  }
})

listen('montage-config-changed', (event) => {
  // Keep other windows current so their later writes preserve collage preferences.
  if (currentWindowLabel !== 'montage') {
    Object.assign(config.montage, event.payload)
  }
})

listen('image-editor-config-changed', (event) => {
  // Keep other windows current so their later writes preserve editor preferences.
  if (currentWindowLabel !== 'imageeditor') {
    Object.assign(config.imageEditor, event.payload)
  }
})

listen('dedup-config-changed', (event) => {
  // Keep other windows current so their later writes preserve dedup keep rules.
  if (currentWindowLabel !== 'main') {
    Object.assign(config.dedup, event.payload)
  }
})

listen('af-area-display-changed', (event) => {
  // All open windows keep the same preference so later store writes preserve it.
  if (typeof event.payload === 'boolean' && config.mediaViewer.showAfAreas !== event.payload) {
    config.mediaViewer.showAfAreas = event.payload
  }
})

// Tile and Masonry styles were removed: fall back to Justified.
if (config.settings.grid.style === 1 || config.settings.grid.style === 3) config.settings.grid.style = 2

// Resolve Regional Format before components first render dates.
await getSystemRegionLocale().then((loc) => config.setSystemLocale(loc)).catch(() => {})

if (isMainWindow) {
  config.$subscribe((_mutation, state) => {
    void emit('config-settings-synced', JSON.parse(JSON.stringify(state.settings)))
  })
  watch(() => config.dedup.keepRules, () => {
    void emit('dedup-config-changed', JSON.parse(JSON.stringify(config.dedup)))
  }, { deep: true })
} else {
  listen('config-settings-synced', async (event) => {
    const request = ++settingsSyncRequest
    if (isSettingsWindow) {
      // Keep shared preferences in sync while preserving other edits in Settings.
      if (event.payload.fileDeleteBehavior === 'trash' || event.payload.fileDeleteBehavior === 'permanent') {
        config.settings.fileDeleteBehavior = event.payload.fileDeleteBehavior
      }
      config.setLanguage(event.payload.language)
    } else {
      const { language, ...settings } = event.payload
      await config.setLanguage(language)
      if (request !== settingsSyncRequest) return
      Object.assign(config.settings, settings)
    }
  })
}

// Load only the active language and English before components read messages.
if (!config.settings.languageInitialized) {
  const systemLanguage = await getSystemLanguage().catch(() => null)
  if (!config.settings.languageInitialized) config.settings.language = resolveSystemLanguage(systemLanguage)
}
const initialLanguage = config.settings.language
const loadedLanguage = await initializeLanguage(initialLanguage)
if (config.settings.language === initialLanguage) {
  config.settings.language = loadedLanguage
  config.settings.languageInitialized = true
}
else i18n.global.locale.value = config.settings.language
setShortcutOverrides(config.settings.shortcutOverrides)
performance.mark('startup-language-ready')
if (isMainWindow) void invoke('record_startup_stage', { stage: 'language-ready' }).catch(console.error)

// Set up global properties
app.config.globalProperties.$invoke = invoke

// Use the router and i18n
app.use(router)
app.use(i18n)

// Mount the app
const originalMediaAvailable = await originalMediaReady
app.mount('#app')
if (!originalMediaAvailable && isMainWindow) {
  void message(i18n.global.t('image_viewer.media_server_unavailable'), { title: 'Lap', kind: 'error' }).catch(console.error)
}
performance.mark('startup-app-mounted')
if (isMainWindow) void invoke('record_startup_stage', { stage: 'app-mounted' }).catch(console.error)

// Listen for events
if (isMainWindow) {
  listen('settings-appearance-changed', (event) => {
    config.setAppearance(event.payload)
  })
  listen('settings-lightTheme-changed', (event) => {
    config.setLightTheme(event.payload)
  })
  listen('settings-darkTheme-changed', (event) => {
    config.setDarkTheme(event.payload)
  })
  listen('settings-scale-changed', (event) => {
    config.setScale(event.payload)
  })
  listen('settings-externalApps-changed', (event) => {
    config.setExternalApps(event.payload)
  })
  listen('settings-language-changed', (event) => {
    config.setLanguage(event.payload, { fromUser: true })
  })
  listen('settings-confirmFileDelete-changed', (event) => {
    config.settings.confirmFileDelete = event.payload !== false
  })
  listen('settings-fileDeleteBehavior-changed', (event) => {
    if (event.payload === 'trash' || event.payload === 'permanent') {
      config.settings.fileDeleteBehavior = event.payload
    }
  })
  listen('settings-showToolTip-changed', (event) => {
    config.setShowToolTip(event.payload)
  })
  listen('settings-autoHideBars-changed', (event) => {
    config.settings.autoHideBars = event.payload
  })
  listen('settings-activateMainOnViewerClose-changed', (event) => {
    config.settings.activateMainOnViewerClose = event.payload
  })
  listen('settings-multipleImageViewers-changed', (event) => {
    config.settings.multipleImageViewers = event.payload
  })
  listen('settings-showStatusBar-changed', (event) => {
    config.setShowStatusBar(event.payload)
  })
  listen('settings-autoCheckUpdates-changed', (event) => {
    config.setAutoCheckUpdates(event.payload)
  })
  listen('settings-debugMode-changed', (event) => {
    config.setDebugMode(event.payload)
  })
  listen('settings-settingsTabIndex-changed', (event) => {
    config.setSettingsTabIndex(event.payload)
  })
  listen('settings-folderSort-changed', (event) => {
    config.setFolderSort(event.payload)
  })
  listen('settings-dateTimeFormat-changed', (event) => {
    config.setDateTimeFormat(event.payload)
  })
  listen('settings-showSubfolderFiles-changed', (event) => {
    config.setShowSubfolderFiles(event.payload)
  })
  listen('settings-thumbnailSize-changed', (event) => {
    config.setThumbnailSize(event.payload)
  })
  listen('settings-rawPairDisplaySource-changed', (event) => {
    config.setRawPairDisplaySource(event.payload)
  })
  listen('settings-rawPreviewSource-changed', (event) => {
    config.setRawPreviewSource(event.payload)
  })
  listen('settings-mapProvider-changed', (event) => {
    config.setMapProvider(event.payload)
  })
  listen('settings-tiandituToken-changed', (event) => {
    config.setTiandituToken(event.payload)
  })
  listen('settings-mapMarkerSize-changed', (event) => {
    config.setMapMarkerSize(event.payload)
  })
  listen('settings-gridStyle-changed', (event) => {
    config.setGridStyle(event.payload)
  })
  listen('settings-gridScaling-changed', (event) => {
    config.setGridScaling(event.payload)
  })
  listen('settings-gridThumbnailCorners-changed', (event) => {
    config.setGridThumbnailCorners(event.payload)
  })
  listen('settings-gridCardBgEnabled-changed', (event) => {
    config.settings.grid.cardBgEnabled = event.payload
  })
  listen('settings-gridCardBgColor-changed', (event) => {
    config.settings.grid.cardBgColor = event.payload
  })
  listen('settings-gridCardAspect-changed', (event) => {
    config.settings.grid.cardAspect = event.payload
  })
  listen('settings-gridLabelLines-changed', (event) => {
    config.settings.grid.labelLines = event.payload
  })
  listen('settings-gridLabelPrimary-changed', (event) => {
    config.setGridLabelPrimary(event.payload)
  })
  listen('settings-gridLabelSecondary-changed', (event) => {
    config.setGridLabelSecondary(event.payload)
  })
  listen('settings-gridThumbnailBadge-changed', (event) => {
    config.setGridThumbnailBadge(event.payload)
  })
  listen('settings-filmStripViewPreviewPosition-changed', (event) => {
    config.setFilmStripViewPreviewPosition(event.payload)
  })
  listen('settings-mouseWheelMode-changed', (event) => {
    config.setMouseWheelMode(event.payload)
  })
  listen('settings-slideShowInterval-changed', (event) => {
    config.setSlideShowInterval(event.payload)
  })
  listen('settings-autoPlayVideo-changed', (event) => {
    config.setAutoPlayVideo(event.payload)
  })
  listen('settings-loopVideo-changed', (event) => {
    config.settings.loopVideo = event.payload
  })
  listen('settings-groupRawJpegPairs-changed', (event) => {
    config.settings.groupRawJpegPairs = event.payload
  })
  listen('settings-navigatorViewMode-changed', (event) => {
    config.setNavigatorViewMode(event.payload)
  })
  listen('settings-navigatorViewSize-changed', (event) => {
    config.setNavigatorViewSize(event.payload)
  })
  listen('settings-navigatorSharpnessGrid-changed', (event) => {
    config.settings.navigatorSharpnessGrid = event.payload
  })
  listen('settings-shortcutOverrides-changed', (event) => {
    config.settings.shortcutOverrides = event.payload
    setShortcutOverrides(event.payload)
  })
  listen('settings-viewBackground-changed', (event) => {
    config.setViewBackground(event.payload)
  })
  listen('settings-hybridDiskCache-changed', (event) => {
    config.settings.hybridDiskCache = event.payload.enabled
    config.settings.hybridDiskCacheAlbumsOnly = event.payload.albumsOnly
  })
  listen('settings-infoTip-changed', (event) => {
    config.settings.infoTipEnabled = event.payload.enabled
    config.settings.infoTipLine1 = event.payload.line1
    config.settings.infoTipLine2 = event.payload.line2
    config.settings.infoTipLine3 = event.payload.line3 ?? ''
  })
  listen('settings-shiftAdvanceMetadata-changed', (event) => {
    config.settings.shiftAdvanceMetadata = event.payload
  })
  listen('settings-thumbnailBackgroundLoad-changed', (event) => {
    config.settings.thumbnailBackgroundLoad = event.payload
  })
  listen('settings-diskKeepBrowseCache-changed', (event) => {
    config.settings.diskKeepBrowseCache = event.payload
  })
  listen('settings-fitNoUpscale-changed', (event) => {
    config.settings.fitNoUpscale = event.payload
  })
  listen('settings-viewerTitleInfo-changed', (event) => {
    config.settings.viewerTitleInfo = event.payload
  })
  listen('settings-treeFontScale-changed', (event) => {
    config.settings.treeFontScale = event.payload
  })
  listen('settings-titleBarHidden-changed', (event) => {
    config.settings.titleBarHidden = event.payload
  })
  listen('settings-hybridFilter-changed', (event) => {
    config.settings.hybridFilter = event.payload
  })
  listen('settings-hybridSharpness-changed', (event) => {
    config.settings.hybridSharpness = event.payload
  })
  listen('settings-hybridTileHoverMs-changed', (event) => {
    config.settings.hybridTileHoverMs = event.payload
  })
  listen('settings-hybridTileHoverRadius-changed', (event) => {
    config.settings.hybridTileHoverRadius = event.payload
  })
  listen('settings-autoRescanOnOpen-changed', (event) => {
    config.settings.autoRescanOnOpen = event.payload
  })
  for (const key of ['accentColor', 'zoomStepAtCursor', 'thumbInnerZoom', 'thumbEnlargePct', 'thumbHoverEnlargePct', 'aiSearchEnabled', 'detectMotionPhotos', 'focusGridAfterAlbumClick', 'showLibrarySidebar', 'showSmartAlbumSidebar', 'showLocationSidebar', 'showCameraSidebar']) {
    listen(`settings-${key}-changed`, (event) => {
      config.settings[key] = event.payload
    })
  }
  listen('settings-infoInWindow-changed', (event) => {
    config.settings.infoInWindow = event.payload
  })
  listen('settings-hybridTileQuality-changed', (event) => {
    config.settings.hybridTileQuality = event.payload
  })
  listen('settings-hybridTileCenter-changed', (event) => {
    config.settings.hybridTileCenter = event.payload
  })
  listen('settings-jpegRam-changed', (event) => {
    config.settings.jpegRam = event.payload
  })
  listen('settings-jpegRamMaxMp-changed', (event) => {
    config.settings.jpegRamMaxMp = event.payload
  })
  for (const key of ['testMtResize', 'testCropInResizer', 'testParSharpen', 'testHalfCopy', 'testRamWait', 'testParFit', 'testRamOnOpen']) {
    listen(`settings-${key}-changed`, (event) => {
      config.settings[key] = event.payload
    })
  }
  listen('settings-jpegRamBudgetMb-changed', (event) => {
    config.settings.jpegRamBudgetMb = event.payload
  })
  listen('settings-hybridTileProgressive-changed', (event) => {
    config.settings.hybridTileProgressive = event.payload
  })
  listen('settings-hybridTilePrefetch-changed', (event) => {
    config.settings.hybridTilePrefetch = event.payload
  })
  listen('settings-hybridTileMegapixels-changed', (event) => {
    config.settings.hybridTileMegapixels = event.payload
  })
  listen('settings-hybridPreview-changed', (event) => {
    config.settings.hybridPreview = event.payload
  })
  listen('settings-pngBackendPreview-changed', (event) => {
    config.settings.pngBackendPreview = event.payload
  })
  listen('settings-panBeyondEdges-changed', (event) => {
    config.settings.panBeyondEdges = event.payload
  })
  listen('settings-stageBeforeShow-changed', (event) => {
    config.settings.stageBeforeShow = event.payload
  })
  listen('settings-loadingBackgroundColor-changed', (event) => {
    config.settings.loadingBackgroundColor = event.payload
  })
  listen('settings-loadTimingFields-changed', (event) => {
    config.settings.loadTimingFields = event.payload
  })
  listen('settings-showLoadTiming-changed', (event) => {
    config.settings.showLoadTiming = event.payload
  })
  listen('settings-albumCoverIcons-changed', (event) => {
    config.settings.albumCoverIcons = event.payload
  })
  listen('settings-scrollbarNarrow-changed', (event) => {
    config.settings.scrollbarNarrow = event.payload
  })
  listen('settings-thumbHighlightBar-changed', (event) => {
    config.settings.thumbHighlightBar = event.payload
  })
  listen('settings-thumbHighlight-changed', (event) => {
    config.settings.thumbHighlightEnabled = event.payload.enabled
    config.settings.thumbHighlightColor = event.payload.color
  })
  listen('settings-rasterWarm-changed', (event) => {
    config.settings.rasterWarm = event.payload
  })
  for (const key of ['transparencyChecker', 'transparencyCheckerA', 'transparencyCheckerB', 'transparencyCheckerSize']) {
    listen(`settings-${key}-changed`, (event) => {
      config.settings[key] = event.payload
    })
  }
  listen('settings-transparencyColor-changed', (event) => {
    config.settings.transparencyColor = event.payload
  })
  listen('settings-slideShowTransition-changed', (event) => {
    config.setSlideShowTransition(event.payload)
  })
  listen('settings-imageSearchThresholdIndex-changed', (event) => {
    config.setImageSearchThresholdIndex(event.payload)
  })
  listen('settings-imageSearchModel-changed', (event) => {
    config.setImageSearchModel(event.payload)
  })
  listen('settings-similarPhotoGroupingThresholdIndex-changed', (event) => {
    config.setSimilarPhotoGroupingThresholdIndex(event.payload)
  })
  listen('settings-faceClusterThresholdIndex-changed', (event) => {
    config.setFaceClusterThresholdIndex(event.payload)
  })
  listen('settings-faceEnabled-changed', (event) => {
    config.setFaceEnabled(event.payload)
  })
  listen('libraries-changed', () => {
    config.notifyLibrariesChanged()
  })
}
