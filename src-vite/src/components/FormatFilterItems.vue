<template>
  <div class="min-w-40" @click.stop>
    <div class="mx-2 my-1 border-t border-base-content/10"></div>
    <button
      class="p-1 w-full flex items-center text-[10px] font-bold uppercase tracking-widest text-base-content/30 hover:text-base-content cursor-pointer"
      :aria-expanded="open"
      @click="open = !open"
    >
      <IconRight class="w-3 h-3 mr-1 shrink-0 transition-transform" :class="open ? 'rotate-90' : ''" />
      <span>{{ $t('toolbar.filter.formats') }}</span>
      <span v-if="modelValue.length > 0" class="ml-auto text-primary normal-case tracking-normal text-xs">{{ modelValue.length === 1 ? modelValue[0] : modelValue.length }}</span>
    </button>
    <template v-if="open">
    <button
      v-if="modelValue.length > 0"
      class="p-1 w-full flex items-center text-sm whitespace-nowrap rounded-box hover:bg-base-100/30 hover:text-base-content cursor-pointer"
      @click="emit('update:modelValue', [])"
    >
      <span class="w-5"></span>
      <span class="mr-4">{{ $t('toolbar.filter.all_formats') }}</span>
    </button>
    <p v-if="loaded && rows.length === 0" class="px-1 py-1 text-xs text-base-content/40">—</p>
    <button
      v-for="row in rows"
      :key="row.label"
      class="p-1 w-full flex items-center text-sm whitespace-nowrap rounded-box hover:bg-base-100/30 hover:text-base-content cursor-pointer"
      role="menuitemcheckbox"
      :aria-checked="modelValue.includes(row.label)"
      @click="toggle(row.label)"
    >
      <span class="w-5"><IconOk v-if="modelValue.includes(row.label)" class="w-4 h-4" /></span>
      <span class="mr-4">{{ row.label }}</span>
      <span class="ml-auto text-[10px] tabular-nums text-base-content/30">{{ row.count.toLocaleString() }}</span>
    </button>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { getQueryFormatCounts } from '@/common/api';
import { IconOk, IconRight } from '@/common/icons';

// Formats present in the current view (all other filters applied) with their file counts. The menu
// content is only mounted while the menu is open, so the counts are loaded on demand.
const props = defineProps<{ modelValue: string[]; params: any }>();
const emit = defineEmits<{ (event: 'update:modelValue', value: string[]): void }>();

const counts = ref<Array<[string, number]>>([]);
const loaded = ref(false);
const open = ref(true);

// Selected formats stay listed even when the current view has no file of that format.
const rows = computed(() => {
  const list = counts.value.map(([label, count]) => ({ label: String(label).toUpperCase(), count: Number(count) }));
  for (const label of props.modelValue) {
    if (!list.some(row => row.label === label)) list.push({ label, count: 0 });
  }
  return list;
});

function toggle(label: string) {
  const next = props.modelValue.includes(label)
    ? props.modelValue.filter(item => item !== label)
    : [...props.modelValue, label];
  emit('update:modelValue', next);
}

onMounted(async () => {
  const result = await getQueryFormatCounts({ ...props.params, formats: [] });
  counts.value = Array.isArray(result) ? result : [];
  loaded.value = true;
});
</script>
