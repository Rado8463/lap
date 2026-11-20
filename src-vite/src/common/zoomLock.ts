import { ref } from 'vue';

// Session-wide "lock zoom": while on, moving to another image keeps the
// current zoom level and position instead of resetting to fit / 100 %.
export const zoomLocked = ref(false);
