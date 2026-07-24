<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
    label?: string;
    suffix?: string;
    disabled?: boolean;
  }>(),
  { min: 0, max: 100, step: 1 },
);

const emit = defineEmits<{ "update:modelValue": [value: number] }>();

const display = computed(() => {
  const v = props.modelValue;
  return props.suffix ? `${v}${props.suffix}` : String(v);
});

function onInput(value: number | null) {
  if (value === null) return;
  emit("update:modelValue", value);
}
</script>

<template>
  <div class="labeled-slider">
    <div class="row">
      <span v-if="label" class="label">{{ label }}</span>
      <span class="value">{{ display }}</span>
    </div>
    <n-slider
      :value="modelValue"
      :min="min"
      :max="max"
      :step="step"
      :disabled="disabled"
      :tooltip="false"
      @update:value="onInput"
    />
  </div>
</template>

<style scoped>
.labeled-slider {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.row {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  font-size: var(--font-size-sm);
}
.label {
  color: var(--text-secondary);
}
.value {
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
}
</style>
