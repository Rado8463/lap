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

// Debug tooltip with the image load timings (D key); the other windows follow through the settings event.
export function toggleLoadTiming() {
  const config = useConfigStore();
  const enabled = config.settings.showLoadTiming !== true;
  config.settings.showLoadTiming = enabled;
  void emit('settings-showLoadTiming-changed', enabled).catch(() => {});
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

// Parts of the debug tooltip with the load timings; the user picks which are shown (default: all).
export const LOAD_TIMING_FIELDS = [
  'cloud', 'fetch', 'decode', 'stage', 'show', 'total', 'size', 'full', 'tile', 'ramMode', 'ramInfo', 'lastTile', 'phases', 'path',
] as const;
export type LoadTimingField = (typeof LOAD_TIMING_FIELDS)[number];

export function parseLoadTimingFields(value: unknown): LoadTimingField[] {
  if (typeof value !== 'string') return [...LOAD_TIMING_FIELDS];
  const wanted = new Set(value.split(',').map((s) => s.trim()));
  if (wanted.has('ram')) { wanted.add('ramMode'); wanted.add('ramInfo'); } // older setting: one RAM part
  return LOAD_TIMING_FIELDS.filter((f) => wanted.has(f));
}

function loadTimingFieldOf(part: string): LoadTimingField | null {
  if (/^cloud-check/.test(part)) return 'cloud';
  if (/^(fetch\+header|preloaded)/.test(part)) return 'fetch';
  if (/^decode /.test(part)) return 'decode';
  if (/^stage /.test(part)) return 'stage';
  if (/^show /.test(part)) return 'show';
  if (/^TOTAL /.test(part)) return 'total';
  if (/^FULL:/.test(part) || /^(waited|painted) /.test(part)) return 'full';
  if (/^TILE /.test(part)) return 'tile';
  if (/^RAM tile:/.test(part)) return 'phases';
  if (/^RAM:/.test(part)) return 'ramMode';
  if (/^last tile:/.test(part)) return 'lastTile';
  if (/^([A-Za-z]:[\\/]|\/)/.test(part)) return 'path';
  if (/^\d+x\d+/.test(part)) return 'size';
  return null;
}

/** Splits "RAM: sequential (no usable ...), 2345 ms, half copy 90 ms" into the mode and the rest. */
function splitRamPart(part: string): { mode: string; info: string } {
  const m = /^RAM: (sequential|parallel)(.*)$/.exec(part);
  if (!m) return { mode: part, info: '' };
  // "(no usable restart markers, or progressive)" only says why it is sequential: dropped.
  let rest = m[2].replace(/^ \(no usable restart markers, or progressive\)/, '');
  rest = rest.replace(/^, /, '');
  return { mode: `RAM: ${m[1]}`, info: rest };
}

/** Keeps only the selected parts of the pipe-separated timing text. */
export function filterLoadTiming(text: string, fields: readonly string[]): string {
  if (!text) return '';
  const allowed = new Set(fields);
  let current: LoadTimingField | null = null;
  const kept: string[] = [];
  for (const part of text.split(' | ')) {
    const field = loadTimingFieldOf(part) ?? current; // unknown pieces belong to the part before them
    current = field;
    if (field === 'ramMode') {
      const { mode, info } = splitRamPart(part);
      if (allowed.has('ramMode')) kept.push(mode);
      if (info && allowed.has('ramInfo')) kept.push(info);
      current = info ? 'ramInfo' : 'ramMode';
      continue;
    }
    if (!field || allowed.has(field)) kept.push(part);
  }
  return kept.join(' | ');
}
