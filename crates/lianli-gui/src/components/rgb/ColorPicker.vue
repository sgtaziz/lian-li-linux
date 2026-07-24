<script setup lang="ts">
import { computed } from "vue";
import type { RGB, RGBA } from "@/types";

const props = withDefaults(
  defineProps<{
    modelValue: RGB | RGBA | null;
    label?: string;
    alpha?: boolean;
    disabled?: boolean;
  }>(),
  { alpha: false, disabled: false },
);

const emit = defineEmits<{
  "update:modelValue": [value: RGB | RGBA];
}>();

// Naive UI color picker works with hex / hex8 / rgb strings.
const stringValue = computed<string>(() => toCss(props.modelValue, props.alpha));

function toCss(c: RGB | RGBA | null, alpha: boolean): string {
  if (!c) return alpha ? "#00000000" : "#000000";
  const [r, g, b] = c;
  const a = c.length === 4 ? (c[3] as number) : 255;
  if (alpha) {
    const hex = `#${hex2(r)}${hex2(g)}${hex2(b)}${hex2(a)}`;
    return hex;
  }
  return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}

function hex2(n: number): string {
  return Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");
}

function fromCss(value: string): RGB | RGBA {
  // value is "#rrggbb" or "#rrggbbaa"
  const r = parseInt(value.slice(1, 3), 16);
  const g = parseInt(value.slice(3, 5), 16);
  const b = parseInt(value.slice(5, 7), 16);
  if (props.alpha && value.length >= 9) {
    const a = parseInt(value.slice(7, 9), 16);
    return [r, g, b, a] as RGBA;
  }
  return [r, g, b];
}

function onUpdate(value: string) {
  emit("update:modelValue", fromCss(value));
}
</script>

<template>
  <div class="color-picker">
    <span v-if="label" class="cp-label">{{ label }}</span>
    <n-color-picker
      :value="stringValue"
      :show-alpha="alpha"
      :modes="['hex']"
      :disabled="disabled"
      size="small"
      @update:value="onUpdate"
    />
  </div>
</template>

<style scoped>
.color-picker {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.cp-label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
</style>
