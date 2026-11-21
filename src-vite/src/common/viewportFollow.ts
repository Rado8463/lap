import { ref } from 'vue';
import type { SavedViewport } from '@/common/savedViewports';

// "Follow viewport" mode of the Image Viewer window: after Save Viewport (V) the current zoom and
// position are carried to every image that is opened next (and updated whenever it changes).
// It lives only in memory, so it ends together with the viewer window.
export const followViewport = ref<Omit<SavedViewport, 'savedAt'> | null>(null);
