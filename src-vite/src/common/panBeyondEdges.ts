import { emit } from '@tauri-apps/api/event';

// Moving a zoomed photo past the screen edges: 0 = off, 1 = vertical only, 2 = free (any direction).
export const PAN_OFF = 0;
export const PAN_VERTICAL = 1;
export const PAN_FREE = 2;

/** Older configs stored a boolean: true meant "allowed", which is the vertical mode now. */
export function normalizePanMode(value: unknown): number {
  if (value === true) return PAN_VERTICAL;
  const n = Number(value);
  return n === PAN_VERTICAL || n === PAN_FREE ? n : PAN_OFF;
}

/** Cycle off -> vertical -> free -> off, tell the other windows, return the new mode. */
export function cyclePanMode(config: any): number {
  const next = (normalizePanMode(config.settings.panBeyondEdges) + 1) % 3;
  config.settings.panBeyondEdges = next;
  void emit('settings-panBeyondEdges-changed', next);
  return next;
}

export const PAN_MODE_LABEL_KEYS = ['pan_mode_off', 'pan_mode_vertical', 'pan_mode_free'];
