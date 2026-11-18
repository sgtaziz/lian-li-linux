<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { ChevronDown, ChevronUp } from "lucide-vue-next";
import { useTranslate } from "@/i18n";

const props = withDefaults(
  defineProps<{
    modelValue: number | null;
    label: string;
    min?: number;
    max?: number;
    step?: number;
    precision?: number;
    unit?: string;
    invalid?: boolean;
    disabled?: boolean;
  }>(),
  { min: 0, max: 100, step: 1, precision: 0 },
);

const emit = defineEmits<{
  "update:modelValue": [value: number];
  commit: [];
}>();

const t = useTranslate();
const input = ref<HTMLInputElement | null>(null);
const focused = ref(false);
const text = ref(format(props.modelValue));

// Telemetry re-renders arrive while the user types, so only resync the text when not editing.
watch(
  () => props.modelValue,
  (value) => {
    if (!focused.value) text.value = format(value);
  },
);

function format(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "";
  return String(round(value));
}

function round(value: number): number {
  const factor = 10 ** props.precision;
  return Math.round(value * factor) / factor;
}

function parse(raw: string): number | null {
  const trimmed = raw.trim().replace(",", ".");
  if (trimmed === "") return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < props.min || value > props.max) return null;
  return round(value);
}

const textInvalid = computed(() => focused.value && parse(text.value) === null);

function onInput(event: Event) {
  text.value = (event.target as HTMLInputElement).value;
}

// Listeners read the new value from their props on commit, which Vue only refreshes after the update.
function commitAfter(changed: boolean) {
  if (changed) void nextTick(() => emit("commit"));
  else emit("commit");
}

function onBlur() {
  focused.value = false;
  const value = parse(text.value);
  const changed = value !== null && value !== props.modelValue;
  if (changed) emit("update:modelValue", value);
  text.value = format(props.modelValue);
  commitAfter(changed);
}

function stepBy(direction: 1 | -1) {
  const base = parse(text.value) ?? props.modelValue ?? props.min;
  const next = Math.min(props.max, Math.max(props.min, round(base + direction * props.step)));
  const changed = next !== props.modelValue;
  text.value = format(next);
  if (changed) emit("update:modelValue", next);
  if (!focused.value) commitAfter(changed);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "ArrowUp" || event.key === "ArrowDown") {
    event.preventDefault();
    stepBy(event.key === "ArrowUp" ? 1 : -1);
  } else if (event.key === "Enter") {
    input.value?.blur();
  }
}
</script>

<template>
  <span class="number-field" :class="{ invalid: invalid || textInvalid, disabled }">
    <input
      ref="input"
      type="text"
      inputmode="decimal"
      class="value"
      :value="text"
      :aria-label="label"
      :aria-invalid="invalid || textInvalid"
      :disabled="disabled"
      @focus="focused = true"
      @input="onInput"
      @blur="onBlur"
      @keydown="onKeydown"
    />
    <span v-if="unit" class="unit">{{ unit }}</span>
    <span class="steppers">
      <button
        type="button"
        class="step"
        tabindex="-1"
        :aria-label="t('common.increaseValue', { label })"
        :disabled="disabled || (modelValue !== null && modelValue >= max)"
        @mousedown.prevent
        @click="stepBy(1)"
      >
        <ChevronUp :size="13" :stroke-width="2.6" />
      </button>
      <button
        type="button"
        class="step"
        tabindex="-1"
        :aria-label="t('common.decreaseValue', { label })"
        :disabled="disabled || (modelValue !== null && modelValue <= min)"
        @mousedown.prevent
        @click="stepBy(-1)"
      >
        <ChevronDown :size="13" :stroke-width="2.6" />
      </button>
    </span>
  </span>
</template>

<style scoped>
.number-field {
  display: flex;
  align-items: center;
  gap: 4px;
  height: 38px;
  min-width: 0;
  padding: 0 4px 0 10px;
  box-sizing: border-box;
  border: 1px solid var(--md-outline);
  border-radius: 10px;
  background: var(--md-surface-container-low);
  transition: border-color 0.15s, box-shadow 0.15s;
}
.number-field:focus-within {
  border-color: var(--md-primary);
  box-shadow: inset 0 0 0 1px var(--md-primary);
}
.number-field.invalid {
  border-color: var(--danger);
  box-shadow: inset 0 0 0 1px var(--danger);
}
.number-field.disabled {
  opacity: 0.5;
}
.value {
  flex: 1;
  width: 100%;
  min-width: 0;
  border: 0;
  outline: none;
  padding: 0;
  background: transparent;
  color: var(--md-on-surface);
  font-family: var(--font-mono);
  font-size: 14px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.unit {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}
.steppers {
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
.step {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 16px;
  padding: 0;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--md-on-surface-variant);
  cursor: pointer;
}
.step:hover:not(:disabled) {
  background: color-mix(in srgb, var(--md-on-surface) 10%, transparent);
  color: var(--md-on-surface);
}
.step:disabled {
  opacity: 0.35;
  cursor: default;
}
</style>
