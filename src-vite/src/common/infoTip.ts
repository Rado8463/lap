import { emit } from '@tauri-apps/api/event';
import { useConfigStore } from '@/stores/configStore';

// Fields of the viewer's info tooltip; the user picks which appear on each of its three lines.
export const INFO_TIP_FIELDS = ['name', 'path', 'dimensions', 'bitDepth', 'size', 'created', 'zoom', 'flags'] as const;
export type InfoTipField = (typeof INFO_TIP_FIELDS)[number];

export const INFO_TIP_DEFAULT_LINE1 = 'name';
export const INFO_TIP_DEFAULT_LINE2 = 'dimensions,bitDepth,size,zoom,flags';
export const INFO_TIP_DEFAULT_LINE3 = '';

export function parseInfoTipFields(value: unknown, fallback: string): InfoTipField[] {
  const raw = typeof value === 'string' ? value : fallback;
  const wanted = new Set(raw.split(',').map((s) => s.trim()));
  return INFO_TIP_FIELDS.filter((field) => wanted.has(field));
}

// Toggle from a keyboard shortcut; the other windows follow through an event.
export function toggleInfoTip() {
  const config = useConfigStore();
  const enabled = config.settings.infoTipEnabled === false;
  config.settings.infoTipEnabled = enabled;
  void emit('settings-infoTip-changed', {
    enabled,
    line1: config.settings.infoTipLine1,
    line2: config.settings.infoTipLine2,
    line3: config.settings.infoTipLine3,
  }).catch(() => {});
}
