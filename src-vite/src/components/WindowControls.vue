<template>
  <div class="h-10 flex items-center shrink-0 -mr-2" @mousedown.stop @dblclick.stop>
    <IconWinMinus
      class="p-3 w-12 h-full text-base-content/70 hover:text-base-content hover:bg-base-100 transition-colors duration-300"
      @click.stop="minimizeWindow"
    />
    <component :is="isMaximized ? IconWinRestore : IconWinMaximize"
      class="p-3 w-12 h-full text-base-content/70 hover:text-base-content hover:bg-base-100 transition-colors duration-300"
      @click.stop="toggleMaximizeWindow"
    />
    <IconClose
      class="p-3 w-12 h-full text-base-content/70 hover:text-base-content hover:bg-red-500 transition-colors duration-300"
      @click.stop="closeWindow"
    />
  </div>
</template>

<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { IconWinMinus, IconWinMaximize, IconWinRestore, IconClose } from '@/common/icons';

const appWindow = getCurrentWindow();
const isMaximized = ref(false);
let unlisten = null;

const refreshMaximized = () => appWindow.isMaximized().then((m) => { isMaximized.value = m; }).catch(() => {});

onMounted(async () => {
  refreshMaximized();
  try { unlisten = await appWindow.onResized(refreshMaximized); } catch { /* ignore */ }
});
onBeforeUnmount(() => { if (unlisten) unlisten(); });

const minimizeWindow = () => { appWindow.minimize(); };
const toggleMaximizeWindow = () => { appWindow.toggleMaximize().then(refreshMaximized); };
const closeWindow = () => { appWindow.close(); };
</script>
