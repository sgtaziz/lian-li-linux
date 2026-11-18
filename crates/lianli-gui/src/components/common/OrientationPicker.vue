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
  border: 1px solid var(--md-outline);
  border-radius: 18px;
  overflow: hidden;
}
.opt {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  height: 34px;
  padding: 0 var(--space-3);
  background: transparent;
  border: 0;
  color: var(--md-on-surface-variant);
  cursor: pointer;
  font: inherit;
  font-size: var(--font-size-sm);
  font-weight: 600;
  transition: background 0.15s, color 0.15s;
}
.opt + .opt {
  border-left: 1px solid var(--md-outline);
}
.opt:hover:not(.active) {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.opt:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -3px;
}
.opt.active {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
</style>
