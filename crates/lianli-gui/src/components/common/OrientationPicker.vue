<script setup lang="ts">
import { computed } from "vue";
import { RotateCw, Square } from "lucide-vue-next";

const props = withDefaults(
  defineProps<{ modelValue: number }>(),
  {},
);
const emit = defineEmits<{ "update:modelValue": [value: number] }>();

const options = [0, 90, 180, 270] as const;

const rotation = computed(() => `rotate(${props.modelValue}deg)`);
</script>

<template>
  <div class="orientation-picker">
    <button
      v-for="deg in options"
      :key="deg"
      type="button"
      class="opt"
      :class="{ active: modelValue === deg }"
      :title="`${deg}\u00B0`"
      @click="emit('update:modelValue', deg)"
    >
      <Square :size="14" :style="{ transform: rotation }" v-if="deg === 0" />
      <RotateCw :size="14" :style="{ transform: `rotate(${deg}deg)` }" v-else />
      <span>{{ deg }}&deg;</span>
    </button>
  </div>
</template>

<style scoped>
.orientation-picker {
  display: inline-flex;
  gap: var(--space-1);
}
.opt {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-2);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-xs);
  transition: all 0.12s;
}
.opt:hover {
  border-color: var(--border-strong);
  color: var(--text-primary);
}
.opt.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent);
}
</style>
