<template>
  <div class="w-screen h-screen flex flex-col overflow-hidden bg-base-300 text-base-content/70">
    <TitleBar v-if="showDesktopTitleBar" :titlebar="titleText" :resizable="true" viewName="Info" class="shrink-0 z-50" />
    <div class="flex-1 min-h-0 overflow-hidden">
      <FileInfo
        :fileInfo="file"
        :empty="!file"
        @close="closeWindow"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emit as tauriEmit, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isWin, isLinux } from '@/common/utils';
import TitleBar from '@/components/TitleBar.vue';
import FileInfo from '@/components/FileInfo.vue';

const appWindow = getCurrentWebviewWindow();
const showDesktopTitleBar = isWin || isLinux;
const file = ref<any>(null);
const titleText = computed(() => file.value?.name ? `Info – ${file.value.name}` : 'Info');
let unlisten: UnlistenFn | null = null;

function closeWindow() {
  void appWindow.close();
}

onMounted(async () => {
  unlisten = await listen('info-window-file', (event: any) => {
    file.value = event.payload?.file ?? null;
  });
  // ask the main window for the current photo
  void tauriEmit('info-window-ready');
  void appWindow.show();
});

onUnmounted(() => { unlisten?.(); });
</script>
