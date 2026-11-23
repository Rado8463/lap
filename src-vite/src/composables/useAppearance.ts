import { computed, onScopeDispose, ref, watch } from 'vue';
import { useConfigStore } from '@/stores/configStore';
import { setTheme } from '@/common/utils';

// Keep the saved mode (0: light, 1: dark, 2: system) separate from the displayed mode.
export function useAppearance() {
  const config = useConfigStore();
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const systemDark = ref(media.matches);
  const onChange = (event: MediaQueryListEvent) => {
    systemDark.value = event.matches;
  };

  media.addEventListener('change', onChange);
  onScopeDispose(() => media.removeEventListener('change', onChange));

  return computed(() => config.settings.appearance === 2
    ? (systemDark.value ? 1 : 0)
    : config.settings.appearance);
}

// App owns theme application for every window, including its loading screen.
export function useAppTheme() {
  const config = useConfigStore();
  const appearance = useAppearance();

  watch(
    [appearance, () => config.settings.lightTheme, () => config.settings.darkTheme],
    ([mode, lightTheme, darkTheme]) => {
      setTheme(mode, mode === 0 ? lightTheme : darkTheme);
    },
    { immediate: true },
  );

  // Custom accent colour overrides the theme's primary colour (inline style beats data-theme).
  watch(
    () => config.settings.accentColor,
    (color) => applyAccentColor(color),
    { immediate: true },
  );
}

export function applyAccentColor(color: string | undefined) {
  const root = document.documentElement;
  const hex = /^#[0-9a-fA-F]{6}$/.test(color || '') ? (color as string) : '';
  if (!hex) {
    root.style.removeProperty('--color-primary');
    root.style.removeProperty('--color-primary-content');
    return;
  }
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  root.style.setProperty('--color-primary', hex);
  root.style.setProperty('--color-primary-content', luminance > 0.6 ? '#000000' : '#ffffff');
}
