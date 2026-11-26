<template>
  <div class="min-w-40" @click.stop>
    <div class="mx-2 my-1 border-t border-base-content/10"></div>
    <button
      class="p-1 w-full flex items-center text-[10px] font-bold uppercase tracking-widest text-base-content/30 hover:text-base-content cursor-pointer"
      :aria-expanded="open"
      @click="open = !open"
    >
      <IconRight class="w-3 h-3 mr-1 shrink-0 transition-transform" :class="open ? 'rotate-90' : ''" />
      <span>{{ title }}</span>
      <span v-if="active" class="ml-auto text-primary normal-case tracking-normal text-xs">{{ summary }}</span>
    </button>
    <template v-if="open">
      <button
        v-for="preset in presets"
        :key="preset.label"
        class="p-1 w-full flex items-center text-sm whitespace-nowrap rounded-box hover:bg-base-100/30 hover:text-base-content cursor-pointer"
        role="menuitemcheckbox"
        :aria-checked="isPresetActive(preset)"
        @click="togglePreset(preset)"
      >
        <span class="w-5"><IconOk v-if="isPresetActive(preset)" class="w-4 h-4" /></span>
        <span class="mr-4">{{ preset.label }}</span>
      </button>
      <div class="p-1 flex items-center gap-1 text-sm">
        <span class="w-4"></span>
        <input
          type="number" min="0" step="any" inputmode="decimal"
          class="input input-bordered input-xs w-16"
          :placeholder="$t('toolbar.filter.range_from')"
          :value="min > 0 ? min : ''"
          @change="onCustom('min', $event)"
          @keydown.enter="($event.target as HTMLInputElement).blur()"
        />
        <span>–</span>
        <input
          type="number" min="0" step="any" inputmode="decimal"
          class="input input-bordered input-xs w-16"
          :placeholder="$t('toolbar.filter.range_to')"
          :value="max > 0 ? max : ''"
          @change="onCustom('max', $event)"
          @keydown.enter="($event.target as HTMLInputElement).blur()"
        />
        <span class="text-xs text-base-content/50">{{ unit }}</span>
      </div>
      <button
        v-if="active"
        class="p-1 w-full flex items-center text-sm whitespace-nowrap rounded-box hover:bg-base-100/30 hover:text-base-content cursor-pointer"
        @click="emit('update', 0, 0)"
      >
        <span class="w-5"></span>
        <span class="mr-4">{{ $t('toolbar.filter.range_clear') }}</span>
      </button>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { IconOk, IconRight } from '@/common/icons';

// A numeric range filter (file size in MB, resolution in MP): a few presets plus free "from - to"
// fields. min is inclusive, max exclusive, 0 means no limit.
interface RangePreset { label: string; min: number; max: number }
const props = defineProps<{ title: string; unit: string; min: number; max: number; presets: RangePreset[] }>();
const emit = defineEmits<{ (event: 'update', min: number, max: number): void }>();

const active = computed(() => props.min > 0 || props.max > 0);
const open = ref(active.value);

const summary = computed(() => {
  if (props.min > 0 && props.max > 0) return `${props.min}–${props.max} ${props.unit}`;
  if (props.min > 0) return `≥ ${props.min} ${props.unit}`;
  return `< ${props.max} ${props.unit}`;
});

function isPresetActive(preset: RangePreset) {
  return preset.min === props.min && preset.max === props.max;
}

function togglePreset(preset: RangePreset) {
  if (isPresetActive(preset)) emit('update', 0, 0);
  else emit('update', preset.min, preset.max);
}

function onCustom(which: 'min' | 'max', event: Event) {
  const value = Math.max(0, Number((event.target as HTMLInputElement).value) || 0);
  const nextMin = which === 'min' ? value : props.min;
  const nextMax = which === 'max' ? value : props.max;
  // An inverted range would show nothing: swap the bounds instead.
  if (nextMin > 0 && nextMax > 0 && nextMin >= nextMax) emit('update', nextMax, nextMin);
  else emit('update', nextMin, nextMax);
}
</script>
