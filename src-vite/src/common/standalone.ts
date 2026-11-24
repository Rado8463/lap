import { ref } from 'vue';

// True in the viewer process started from Windows Explorer ("Open with"): it shows files that are
// not in a library, and has no zoom lock and no saved viewports.
export const standaloneViewer = ref(false);
