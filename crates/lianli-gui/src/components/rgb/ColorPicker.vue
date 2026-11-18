<script setup lang="ts">
import { computed } from "vue";
import type { RGB, RGBA } from "@/types";
import { useTranslate } from "@/i18n";
import { PRESET_COLORS, recentColors, rememberColor } from "./recentColors";

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

const t = useTranslate();

const stringValue = computed<string>(() => toCss(props.modelValue, props.alpha));
let valueWhenOpened = "";
let open = false;

function toCss(c: RGB | RGBA | null, alpha: boolean): string {
  if (!c) return alpha ? "#00000000" : "#000000";
  const [r, g, b] = c;
  const a = c.length === 4 ? (c[3] as number) : 255;
  if (alpha) return `#${hex2(r)}${hex2(g)}${hex2(b)}${hex2(a)}`;
  return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}

function hex2(n: number): string {
  return Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");
}

function fromCss(value: string): RGB | RGBA {
  const r = parseInt(value.slice(1, 3), 16);
  const g = parseInt(value.slice(3, 5), 16);
  const b = parseInt(value.slice(5, 7), 16);
  if (props.alpha) {
    const a = value.length >= 9 ? parseInt(value.slice(7, 9), 16) : 255;
    return [r, g, b, a] as RGBA;
  }
  return [r, g, b];
}

function onUpdate(value: string) {
  emit("update:modelValue", fromCss(value));
}

function onShow(show: boolean) {
  if (show === open) return;
  open = show;
  if (show) {
    valueWhenOpened = stringValue.value;
  } else if (stringValue.value !== valueWhenOpened) {
    rememberColor(stringValue.value.slice(0, 7));
  }
}

function pickSwatch(hex: string) {
  const alphaSuffix = props.alpha ? stringValue.value.slice(7, 9) || "ff" : "";
  onUpdate(hex + alphaSuffix);
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
      @update:show="onShow"
    >
      <template #trigger="{ onClick, ref: setTriggerRef }">
        <button
          :ref="setTriggerRef"
          type="button"
          class="cp-trigger"
          :disabled="disabled"
          :aria-label="label ? t('rgb.colorPicker.chooseNamed', { label, color: stringValue }) : t('rgb.colorPicker.choose', { color: stringValue })"
          @click="onClick"
        >
          <span class="cp-swatch" :class="{ checker: alpha }"><span :style="{ background: stringValue }" /></span>
          <span class="cp-hex">{{ stringValue }}</span>
        </button>
      </template>
      <template #action>
        <div class="cp-recent">
          <div class="cp-recent-list" role="group" :aria-label="t('rgb.colorPicker.presets')">
            <button
              v-for="hex in PRESET_COLORS"
              :key="hex"
              type="button"
              class="cp-recent-swatch"
              :style="{ background: hex }"
              :aria-label="t('rgb.colorPicker.use', { color: hex })"
              :title="hex"
              @click="pickSwatch(hex)"
            />
          </div>
          <template v-if="recentColors.length">
            <span class="cp-recent-label">{{ t("rgb.colorPicker.recent") }}</span>
            <div class="cp-recent-list">
              <button
                v-for="hex in recentColors"
                :key="hex"
                type="button"
                class="cp-recent-swatch"
                :style="{ background: hex }"
                :aria-label="t('rgb.colorPicker.use', { color: hex })"
                :title="hex"
                @click="pickSwatch(hex)"
              />
            </div>
          </template>
        </div>
      </template>
    </n-color-picker>
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
.cp-trigger {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  align-self: flex-start;
  height: 34px;
  padding: 0 var(--space-3) 0 4px;
  border: 1px solid var(--md-outline);
  border-radius: 17px;
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  cursor: pointer;
  transition: background 0.15s;
}
.cp-trigger:hover:not(:disabled) {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
}
.cp-trigger:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 2px;
}
.cp-trigger:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
.cp-swatch {
  display: block;
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  overflow: hidden;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--md-on-surface) 22%, transparent);
}
.cp-swatch > span {
  display: block;
  width: 100%;
  height: 100%;
}
.cp-swatch.checker {
  background: repeating-conic-gradient(#bbb 0 25%, #fff 0 50%) 50% / 10px 10px;
}
.cp-hex {
  font-family: var(--font-mono);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.cp-recent {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 100%;
}
.cp-recent-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.cp-recent-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.cp-recent-swatch {
  width: 22px;
  height: 22px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--md-on-surface) 25%, transparent);
  cursor: pointer;
}
.cp-recent-swatch:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 2px;
}
</style>
