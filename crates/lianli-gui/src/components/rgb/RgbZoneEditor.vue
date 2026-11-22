<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { Check, Plus, RotateCcw, Trash2 } from "lucide-vue-next";
import type { RgbDeviceCapabilities, RgbZoneConfig, RgbEffect, RGB } from "@/types";
import { useRgbStore } from "@/stores/rgb";
import { useConfigStore } from "@/stores/config";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import LedStrip from "@/components/rgb/LedStrip.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import { recallDeviceEffect, rememberDeviceEffect } from "@/components/rgb/effectMemory";
import { RGB_DIRECTIONS, RGB_BRIGHTNESS, modeLabel, scopeLabel } from "@/constants";
import { useTranslate } from "@/i18n";
import { backendText } from "@/utils/backendText";

const props = defineProps<{
  deviceId: string;
  cap: RgbDeviceCapabilities;
  zoneIndex: number;
  zone: RgbZoneConfig;
  directOnly?: boolean;
  groupEffects?: boolean;
  perFanOnly?: boolean;
  mergedPorts?: boolean;
  lightingLocked?: boolean;
  title?: string;
}>();
const emit = defineEmits<{ change: [] }>();

const t = useTranslate();
const rgb = useRgbStore();
const config = useConfigStore();
const dialog = useDialog();

const expanded = ref(true);

const effect = computed(() => props.zone.effect);

const isDirect = computed(() => props.directOnly || effect.value.mode === "Direct");
const hasSoftwareRenderer = computed(() => (props.cap.software_modes?.length ?? 0) > 0);
const isSoftware = computed(() => props.cap.software_modes?.includes(effect.value.mode) ?? false);
const isAnimated = computed(() => !["Off", "Static", "Direct"].includes(effect.value.mode));
const canEditLeds = computed(() => props.cap.supports_direct && hasSoftwareRenderer.value);
const countControl = computed(() => props.cap.fan_led_count_control?.zone === props.zoneIndex ? props.cap.fan_led_count_control : null);
const ledCount = computed(() => countControl.value
  ? config.rgbDeviceConfig(props.deviceId).fan_led_count ?? countControl.value.default
  : props.cap.zones[props.zoneIndex]?.led_count ?? 0);

function onLedCount(value: number) {
  config.rgbDeviceConfig(props.deviceId).fan_led_count = value;
  config.markDirty();
}
const ledColors = ref<RGB[]>([]);
const selectedLeds = ref<number[]>([]);

async function loadLedColors() {
  if (isDirect.value && canEditLeds.value && ledCount.value > 0) {
    ledColors.value = await rgb.getZoneColors(props.deviceId, props.zoneIndex);
    if (ledColors.value.length === 0) {
      ledColors.value = Array.from({ length: ledCount.value }, () => [0, 0, 0] as RGB);
    }
  }
}

watch(
  () => [isDirect.value, canEditLeds.value, ledCount.value, props.zoneIndex] as const,
  () => void loadLedColors(),
  { immediate: true },
);

const supportedScopes = computed(
  () => props.cap.supported_scopes?.[props.zoneIndex] ?? [],
);
const showScope = computed(() => !props.perFanOnly && !isSoftware.value && supportedScopes.value.length > 0);
const modeParameters = computed(() =>
  isSoftware.value ? undefined : props.cap.effect_parameters?.find((entry) => entry.mode === effect.value.mode),
);
const maxColors = computed(() => Math.min(4, modeParameters.value?.max_colors ?? 4));
const showSpeed = computed(() => isAnimated.value && (modeParameters.value?.supports_speed ?? true));
const showDirection = computed(
  () => (isSoftware.value && isAnimated.value) || (props.cap.supports_direction && (modeParameters.value?.directions.length ?? 1) > 0),
);

const propagateChoice = ref<"ask" | "yes" | "no">("ask");

function propagateToZones() {
  const devCfg = config.rgbDeviceConfig(props.deviceId);
  for (let i = 1; i < devCfg.zones.length; i++) {
    devCfg.zones[i].effect = JSON.parse(JSON.stringify(effect.value));
  }
}

function patchEffect(p: Partial<RgbEffect>) {
  if (props.mergedPorts) {
    Object.assign(effect.value, p, { scope: "All" });
    emit("change");
    return;
  }
  if (props.perFanOnly) {
    const device = config.rgbDeviceConfig(props.deviceId);
    for (const zone of device.zones) {
      zone.effect = { ...zone.effect, scope: "Fan", mode: props.cap.zone_effect_modes?.includes(zone.effect.mode) ? zone.effect.mode : "Static" };
    }
    p.scope = "Fan";
  }
  if (props.groupEffects && effect.value.scope === "Fan") p.scope ??= "All";
  Object.assign(effect.value, p);

  if (props.groupEffects) propagateToZones();

  if (props.zoneIndex === 0 && !hasSoftwareRenderer.value && !props.groupEffects && !props.perFanOnly) {
    const isPerFan =
      (effect.value.mode === "Off" ||
        effect.value.mode === "Static" ||
        effect.value.mode === "Direct") &&
      effect.value.scope === "All";
    if (!isPerFan) {
      if (propagateChoice.value === "yes") {
        propagateToZones();
      } else if (propagateChoice.value === "ask") {
        dialog.warning({
          title: t("rgb.zoneEditor.applyAllTitle"),
          content: t("rgb.zoneEditor.applyAllContent"),
          positiveText: t("rgb.zoneEditor.applyAll"),
          negativeText: t("rgb.zoneEditor.zoneZeroOnly"),
          onPositiveClick: () => {
            propagateChoice.value = "yes";
            propagateToZones();
          },
          onNegativeClick: () => {
            propagateChoice.value = "no";
          },
        });
      }
    }
  }

  // Clear any active preset — the user is manually overriding the effect, so
  // the daemon must not re-push stale preset LED colors on save.
  config.rgbDeviceConfig(props.deviceId).active_preset = null;
  config.markDirty();
}

// Devices with long mode lists keep a dropdown so the chips do not crowd the card.
const MAX_MODE_CHIPS = 14;
const currentMode = computed(() => (props.directOnly ? "Direct" : effect.value.mode));
const brightnessSegments = computed(() =>
  RGB_BRIGHTNESS.map((option) => ({ value: String(option.value), label: option.label })),
);
const directionSegments = computed(() =>
  RGB_DIRECTIONS.map((option) => ({ value: option.value, label: option.label })),
);

function modeOptions() {
  const modes = props.directOnly ? ["Direct"] : props.groupEffects ? props.cap.group_effect_modes ?? [] : props.perFanOnly ? props.cap.zone_effect_modes ?? [] : props.cap.supported_modes;
  return modes.map((m) => ({
    label: modeLabel(m),
    value: m,
  }));
}

function onMode(value: string) {
  if (effect.value.mode === value && (!props.perFanOnly || effect.value.scope === "Fan") && (!props.groupEffects || effect.value.scope !== "Fan")) return;
  if (props.mergedPorts) return patchEffect({ mode: value });
  const device = config.rgbDeviceConfig(props.deviceId);
  const memoryZone = props.groupEffects ? null : props.zoneIndex;
  rememberDeviceEffect(device, memoryZone, effect.value, false);
  const scope = props.perFanOnly ? "Fan" : props.cap.software_modes?.includes(value) || (props.groupEffects && effect.value.scope === "Fan") ? "All" : effect.value.scope;
  const remembered = recallDeviceEffect(device, memoryZone, scope, value);
  patchEffect(remembered?.effect ?? { mode: value, scope });
}

function onColor(index: number, value: RGB | any) {
  const next = [...effect.value.colors];
  next[index] = value as RGB;
  patchEffect({ colors: next });
}

function addColor() {
  patchEffect({ colors: [...effect.value.colors, [255, 255, 255]] });
}
function removeColor(index: number) {
  if (effect.value.colors.length <= 1) return;
  dialog.error({
    title: t("rgb.zoneEditor.removeColorTitle"),
    content: t("rgb.zoneEditor.removeColorContent", { index: index + 1 }),
    positiveText: t("common.remove"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => {
      const next = effect.value.colors.filter((_, i) => i !== index);
      patchEffect({ colors: next });
    },
  });
}

function onDirection(value: any) {
  patchEffect({ direction: value });
}
function onScope(value: any) {
  patchEffect({ scope: value });
}
function onSpeed(value: number) {
  patchEffect({ speed: value });
}
function onBrightness(value: number) {
  patchEffect({ brightness: value });
}
function onSwapLr(v: boolean) {
  props.zone.swap_lr = v;
  config.markDirty();
}
function onSwapTb(v: boolean) {
  props.zone.swap_tb = v;
  config.markDirty();
}

function onSelectLed(i: number) {
  const idx = selectedLeds.value.indexOf(i);
  if (idx >= 0) selectedLeds.value.splice(idx, 1);
  else selectedLeds.value.push(i);
}

const directColor = ref<RGB>([255, 255, 255]);
function setDirectColor(value: RGB | any) {
  directColor.value = value as RGB;
}

function markDirectChange() {
  const device = config.rgbDeviceConfig(props.deviceId);
  device.active_preset = null;
  if (props.directOnly) {
    device.regions = null;
    for (const zone of device.zones) zone.effect.mode = "Direct";
  }
  config.markDirty();
}

function applyDirect() {
  if (selectedLeds.value.length === 0) return;
  for (const i of selectedLeds.value) {
    ledColors.value[i] = directColor.value;
  }
  selectedLeds.value = [];
  markDirectChange();
  void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
}

function fillAll() {
  ledColors.value = Array.from({ length: ledCount.value }, () => directColor.value);
  markDirectChange();
  void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
}

function clearAll() {
  dialog.error({
    title: t("rgb.zoneEditor.clearAllTitle"),
    content: t("rgb.zoneEditor.clearAllContent"),
    positiveText: t("rgb.zoneEditor.clear"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => {
      ledColors.value = Array.from({ length: ledCount.value }, () => [0, 0, 0]);
      markDirectChange();
      void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
    },
  });
}

const zoneLabel = computed(() => {
  if (props.title) return props.title;
  if (props.mergedPorts) return t("rgb.zoneEditor.mergedEffects");
  if (props.groupEffects) return t("rgb.zoneEditor.portEffects");
  const zone = props.cap.zones[props.zoneIndex];
  return zone ? backendText(zone.name, zone.name_ui) : t("rgb.zoneEditor.zoneName", { index: props.zoneIndex });
});
</script>

<template>
  <div class="zone" :class="{ collapsed: !expanded }">
    <div class="zone-head" @click="expanded = !expanded">
      <span class="zone-name">{{ zoneLabel }}</span>
      <span class="muted">{{ t("rgb.zoneEditor.ledCount", { count: ledCount }) }}</span>
    </div>

    <div v-if="expanded" class="zone-body">
      <details v-if="countControl" class="led-count-settings">
        <summary>{{ t("rgb.zoneEditor.configureLedCount") }}</summary>
        <div class="led-count-row">
          <LabeledSlider class="led-count-slider" :label="t('rgb.zoneEditor.ledCountLabel')" :model-value="ledCount" :min="countControl.min" :max="countControl.max" :step="1" @update:model-value="onLedCount" />
          <n-button v-if="ledCount !== countControl.default" size="tiny" quaternary :title="t('rgb.zoneEditor.resetTo', { value: countControl.default })" :aria-label="t('rgb.zoneEditor.resetLedCountTo', { value: countControl.default })" @click="onLedCount(countControl.default)">
            <template #icon><RotateCcw :size="14" /></template>
          </n-button>
        </div>
      </details>
      <div class="effect-controls" :class="{ 'sync-locked': lightingLocked }" :inert="lightingLocked || undefined">
      <div class="field">
        <label class="muted">{{ t("rgb.effect.mode") }}</label>
        <div v-if="modeOptions().length <= MAX_MODE_CHIPS" class="mode-chips" role="group" :aria-label="t('rgb.effect.mode')">
          <button
            v-for="option in modeOptions()"
            :key="option.value"
            type="button"
            class="mode-chip"
            :class="{ selected: option.value === currentMode }"
            :aria-pressed="option.value === currentMode"
            @click="onMode(option.value)"
          >
            <Check v-if="option.value === currentMode" :size="14" :stroke-width="2.6" />
            {{ option.label }}
          </button>
        </div>
        <n-select
          v-else
          size="small"
          :value="currentMode"
          :options="modeOptions()"
          @update:value="onMode"
          style="max-width: 260px"
        />
      </div>

      <p v-if="mergedPorts" class="muted">{{ t("rgb.zoneEditor.mergedEffectsHint") }}</p>
      <p v-else-if="groupEffects" class="muted">{{ t("rgb.zoneEditor.groupEffectsHint") }}</p>
      <p v-else-if="perFanOnly" class="muted">{{ t("rgb.zoneEditor.perFanHint") }}</p>
      <p v-else-if="directOnly" class="muted">{{ t("rgb.zoneEditor.directHint") }}</p>
      <p v-else-if="hasSoftwareRenderer" class="muted">{{ t("rgb.zoneEditor.softwareHint") }}</p>

      <div v-if="isDirect && canEditLeds && ledCount > 0" class="direct">
        <LedStrip
          :colors="ledColors"
          :selected="selectedLeds"
          :count="ledCount"
          @select="onSelectLed"
        />
        <div class="direct-controls">
          <ColorPicker class="direct-color" :model-value="directColor" :label="t('rgb.effect.color')" @update:model-value="setDirectColor" />
          <n-button size="small" :disabled="!selectedLeds.length" @click="applyDirect">{{ t("common.apply") }}</n-button>
          <n-button size="small" @click="fillAll">{{ t("rgb.zoneEditor.fillAll") }}</n-button>
          <n-button size="small" @click="clearAll">{{ t("rgb.zoneEditor.clear") }}</n-button>
        </div>
      </div>

      <div v-if="!isDirect && maxColors > 0 && !(isSoftware && ['Off', 'Rainbow', 'RainbowMorph'].includes(effect.mode))" class="colors">
        <label class="muted">{{ t("rgb.effect.colors") }}</label>
        <div class="color-list">
          <div v-for="(_, i) in effect.colors" :key="i" class="color-item">
            <ColorPicker
              class="zone-color"
              :model-value="effect.colors[i]"
              @update:model-value="(v) => onColor(i, v)"
            />
            <n-button quaternary size="small" type="error" @click="removeColor(i)">
              <template #icon><Trash2 :size="14" /></template>
            </n-button>
          </div>
          <n-button size="small" quaternary @click="addColor" v-if="effect.colors.length < maxColors">
            <template #icon><Plus :size="14" /></template>
          </n-button>
        </div>
      </div>

      <div v-if="!isDirect" class="two-col">
        <LabeledSlider
          v-if="showSpeed"
          :label="t('rgb.effect.speed')"
          :model-value="effect.speed"
          :min="0"
          :max="4"
          @update:model-value="onSpeed"
        />
        <div class="field">
          <label class="muted">{{ t("rgb.effect.brightness") }}</label>
          <SegmentedControl
            size="small"
            :model-value="String(effect.brightness)"
            :options="brightnessSegments"
            :label="t('rgb.effect.brightness')"
            @update:model-value="(value) => onBrightness(Number(value))"
          />
        </div>
      </div>

      <div v-if="showDirection || showScope" class="two-col">
        <div v-if="showDirection" class="field">
          <label class="muted">{{ t("rgb.effect.direction") }}</label>
          <SegmentedControl
            size="small"
            :model-value="effect.direction"
            :options="directionSegments"
            :label="t('rgb.effect.direction')"
            @update:model-value="onDirection"
          />
        </div>
        <div v-if="showScope" class="field">
          <label class="muted">{{ t("rgb.effect.scope") }}</label>
          <n-select
            size="small"
            :value="groupEffects && effect.scope === 'Fan' ? 'All' : effect.scope"
            :options="supportedScopes.map((s) => ({ label: scopeLabel(s), value: s }))"
            @update:value="onScope"
          />
        </div>
      </div>

      <div v-if="cap.supports_direction && !groupEffects" class="checkboxes">
        <n-checkbox :checked="zone.swap_lr" @update:checked="onSwapLr">{{ t("rgb.zoneEditor.swapLr") }}</n-checkbox>
        <n-checkbox :checked="zone.swap_tb" @update:checked="onSwapTb">{{ t("rgb.zoneEditor.swapTb") }}</n-checkbox>
      </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.effect-controls { display: flex; flex-direction: column; gap: var(--space-3); }
.sync-locked { opacity: 0.5; }
.led-count-settings > summary {
  cursor: pointer;
  color: var(--md-on-surface-variant);
  font-size: 12px;
}
.led-count-settings[open] > summary {
  margin-bottom: var(--space-2);
}
.led-count-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.led-count-slider { flex: 1; min-width: 0; }
.zone {
  border-radius: 22px;
  background: var(--md-surface-container);
  overflow: hidden;
}
.zone-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 14px 20px;
  cursor: pointer;
}
.zone-head:hover {
  background: color-mix(in srgb, var(--md-on-surface) 5%, transparent);
}
.zone-name {
  font-size: 15px;
  font-weight: 600;
}
.zone-body {
  padding: 0 20px 20px;
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.field > label.muted,
.colors > label.muted {
  font-size: var(--font-size-xs);
  font-weight: 700;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}
.mode-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.mode-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 32px;
  padding: 0 var(--space-3);
  border: 1px solid var(--md-outline);
  border-radius: 10px;
  background: transparent;
  color: var(--md-on-surface-variant);
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.mode-chip:hover:not(.selected) {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.mode-chip.selected {
  border-color: var(--md-secondary-container);
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.mode-chip:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 1px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.direct {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.direct-controls {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.direct-controls .direct-color {
  min-width: 160px;
}
.colors {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.color-list {
  display: flex;
  gap: var(--space-2);
  align-items: flex-end;
  flex-wrap: wrap;
}
.color-item {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
/* Give the picker a definite width so its color swatch renders (in a flex row
   it otherwise collapses, showing only the hex text — same bug as gauge ranges). */
.color-item .zone-color {
  min-width: 130px;
}
.two-col {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 380px), 1fr));
  gap: var(--space-4);
}
.field > .segmented {
  align-self: flex-start;
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.checkboxes {
  display: flex;
  gap: var(--space-4);
}
</style>
