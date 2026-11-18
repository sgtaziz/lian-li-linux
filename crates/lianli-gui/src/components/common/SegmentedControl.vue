<script setup lang="ts" generic="T extends string">
import { ref, useId } from "vue";
import { Check, Info } from "lucide-vue-next";

defineProps<{
  modelValue: T | null;
  options: { value: T; label: string; disabled?: boolean; title?: string }[];
  label: string;
  size?: "small" | "medium";
  disabled?: boolean;
}>();

const emit = defineEmits<{ "update:modelValue": [value: T] }>();
const reasonId = useId();
const explaining = ref<string | null>(null);
</script>

<template>
  <div class="segmented" :class="size ?? 'medium'" role="group" :aria-label="label">
    <!-- Unavailable options stay focusable so their reason can be read on hover or focus. -->
    <n-tooltip
      v-for="(option, index) in options"
      :key="option.value"
      trigger="manual"
      :show="explaining === option.value"
      :disabled="!(option.disabled && option.title)"
    >
      <template #trigger>
        <button
          type="button"
          class="segment"
          :class="{ selected: option.value === modelValue, unavailable: option.disabled, explained: option.disabled && option.title }"
          :aria-pressed="option.value === modelValue"
          :aria-disabled="option.disabled || undefined"
          :aria-describedby="option.disabled && option.title ? `${reasonId}-${index}` : undefined"
          :disabled="disabled"
          :title="option.disabled ? undefined : option.title"
          @click="option.disabled || emit('update:modelValue', option.value)"
          @focus="explaining = option.value"
          @mouseenter="explaining = option.value"
          @blur="explaining = null"
          @mouseleave="explaining = null"
        >
          <Check v-if="option.value === modelValue" :size="size === 'small' ? 13 : 15" :stroke-width="2.6" />
          <span>{{ option.label }}</span>
          <Info v-if="option.disabled && option.title" class="reason-icon" :size="size === 'small' ? 13 : 14" :stroke-width="2.2" />
        </button>
      </template>
      {{ option.title }}
    </n-tooltip>
    <template v-for="(option, index) in options" :key="`reason-${option.value}`">
      <span v-if="option.disabled && option.title" :id="`${reasonId}-${index}`" class="reason-text">{{ option.title }}</span>
    </template>
  </div>
</template>

<style scoped>
.segmented {
  display: inline-flex;
  max-width: 100%;
  border: 1px solid var(--md-outline);
  border-radius: 20px;
  overflow-x: auto;
  scrollbar-width: none;
}
.segment {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: 36px;
  padding: 0 14px;
  border: 0;
  background: transparent;
  color: var(--md-on-surface-variant);
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.small .segment {
  height: 30px;
  padding: 0 11px;
  font-size: 12px;
}
.segment + .segment {
  border-left: 1px solid var(--md-outline);
}
.segment:hover:not(:disabled):not(.selected):not(.unavailable) {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.segment:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -3px;
}
.segment.selected {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.segment:disabled,
.segment.unavailable {
  cursor: not-allowed;
}
.segment:disabled > span,
.segment.unavailable > span {
  opacity: 0.45;
}
.segment.explained {
  cursor: help;
}
.reason-icon {
  opacity: 0.7;
}
.reason-text {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}
.segment:disabled {
  opacity: 0.45;
}
</style>
