<script setup lang="ts">
import { computed } from "vue";
import type { DeviceInfo, AioConfig, FanSpeed } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import NumberField from "@/components/common/NumberField.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import SensorSelect from "@/components/common/SensorSelect.vue";
import { Droplet, Fan, Thermometer } from "lucide-vue-next";
import { useThemeStore } from "@/stores/theme";
import { enumerateSensorsAsOptions, optionForConfig, decodeOption } from "@/stores/sensorOptions";
import { useTranslate } from "@/i18n";

const t = useTranslate();
const props = defineProps<{ device: DeviceInfo }>();

const config = useConfigStore();
const devices = useDevicesStore();
const names = useDeviceNamesStore();
const theme = useThemeStore();

const aio = computed<AioConfig>(() => config.aioConfigFor(props.device.device_id));
const fanCount = computed(() => props.device.fan_count ?? 0);
const pumpControl = computed(() => props.device.has_pump_control ?? props.device.has_pump ?? false);
const hasFan = computed(() => props.device.has_fan ?? false);

const pumpRpm = computed(() => {
  const rpms = devices.fanRpms(props.device.device_id);
  return rpms[fanCount.value] ?? null;
});
const fanRpms = computed(() => {
  const rpms = devices.fanRpms(props.device.device_id);
  return rpms.slice(0, fanCount.value);
});
const coolant = computed(() => devices.coolantTemp(props.device.device_id));

const pumpMode = computed(() => speedMode(aio.value.pump_target_rpm));

function speedMode(s: FanSpeed): string {
  if (typeof s === "number") return "constant";
  if (s === "__mb_sync__" || s.startsWith("__mb_sync__:")) return "__mb_sync__";
  if (s === "off" || s === "") return "off";
  return `curve:${s}`;
}

function onPumpMode(value: string) {
  aio.value.pump_target_rpm = decodeMode(value, aio.value.pump_target_rpm);
  config.markDirty();
}
function onFanMode(slot: number, value: string) {
  const next = [...aio.value.fan_speeds] as FanSpeed[];
  next[slot] = decodeMode(value, next[slot]);
  aio.value.fan_speeds = next;
  config.markDirty();
}
function decodeMode(value: string, current: FanSpeed): FanSpeed {
  if (value === "off") return "off";
  if (value === "constant") return typeof current === "number" ? current : 128;
  if (value === "__mb_sync__") return "__mb_sync__";
  if (value.startsWith("curve:")) return value.slice("curve:".length);
  return current;
}

function pumpPwm(): number {
  const s = aio.value.pump_target_rpm;
  return typeof s === "number" ? s : 128;
}
function setPumpPwm(v: number) {
  aio.value.pump_target_rpm = v;
  config.markDirty();
}
function fanPwm(slot: number): number {
  const s = aio.value.fan_speeds[slot];
  return typeof s === "number" ? s : 128;
}
function setFanPwm(slot: number, v: number) {
  const next = [...aio.value.fan_speeds] as FanSpeed[];
  next[slot] = v;
  aio.value.fan_speeds = next;
  config.markDirty();
}

type ModeKind = "curve" | "fixed" | "mb" | "off";

function kindOf(mode: string): ModeKind {
  if (mode === "constant") return "fixed";
  if (mode === "__mb_sync__") return "mb";
  if (mode === "off") return "off";
  return "curve";
}

function kindOptions(mbSync: boolean) {
  const hasCurves = config.config.fan_curves.length > 0;
  return [
    {
      value: "curve" as const,
      label: t("fans.groupCard.modeCurve"),
      disabled: !hasCurves,
      title: hasCurves ? undefined : t("fans.groupCard.noCurves"),
    },
    { value: "fixed" as const, label: t("fans.groupCard.modeFixed") },
    ...(mbSync ? [{ value: "mb" as const, label: t("fans.groupCard.modeMbSync") }] : []),
    { value: "off" as const, label: t("common.off") },
  ];
}

const pumpKindOptions = computed(() => kindOptions(Boolean(props.device.pump_mb_sync_support)));
const fanKindOptions = computed(() => kindOptions(props.device.mb_sync_support));

function kindValue(kind: ModeKind, currentMode: string): string {
  if (kind === "curve") {
    return currentMode.startsWith("curve:") ? currentMode : `curve:${config.config.fan_curves[0]?.name ?? ""}`;
  }
  if (kind === "fixed") return "constant";
  if (kind === "mb") return "__mb_sync__";
  return "off";
}

function onPumpKind(kind: ModeKind) {
  if (kindOf(pumpMode.value) !== kind) onPumpMode(kindValue(kind, pumpMode.value));
}

function onFanKind(slot: number, kind: ModeKind) {
  const current = speedMode(aio.value.fan_speeds[slot]);
  if (kindOf(current) !== kind) onFanMode(slot, kindValue(kind, current));
}

const curveNames = computed(() => config.config.fan_curves.map((c) => c.name));
const fanSlots = computed(() => (fanCount.value > 0 ? Math.min(fanCount.value, 4) : 0));

function percentOf(pwm: number): number {
  return Math.round((pwm / 255) * 100);
}
function pwmOf(percent: number): number {
  return Math.round((percent / 100) * 255);
}

const fixedSliderTheme = computed(() => ({
  fillColor: theme.palette.tertiary,
  fillColorHover: theme.palette.tertiary,
}));

function coolantTone(celsius: number): string {
  if (celsius < 35) return "cool";
  if (celsius < 45) return "warm";
  return "hot";
}

const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, false));

function sensorValue(key: "cpu_temp_source" | "cpu_load_source" | "gpu_temp_source" | "gpu_load_source") {
  return optionForConfig(config.sensors, aio.value[key]);
}
function onSensor(key: "cpu_temp_source" | "cpu_load_source" | "gpu_temp_source" | "gpu_load_source", value: string) {
  aio.value[key] = decodeOption(value);
  config.markDirty();
}

const rotationSegments = [
  { value: "0", label: "0°" },
  { value: "1", label: "90°" },
  { value: "2", label: "180°" },
  { value: "3", label: "270°" },
];
function onRotation(value: string) {
  aio.value.rotation = Number(value);
  config.markDirty();
}

const themeOptions = computed(() => Array.from({ length: 13 }, (_, i) => ({
  label: t("devices.aio.themeOption", { index: i }),
  value: i,
})));

function setColor(key: "str_color" | "val_color" | "unit_color", v: any) {
  aio.value[key] = v;
  config.markDirty();
}

const mac = computed(() =>
  props.device.device_id.startsWith("wireless:")
    ? props.device.device_id.slice("wireless:".length)
    : props.device.device_id,
);
</script>

<template>
  <div class="aio">
    <section class="hero">
      <div class="hero-main">
        <span class="hero-name">{{ names.label(device.device_id, device.name) }}</span>
        <span class="hero-id mono">{{ mac }}</span>
      </div>
      <div class="stats">
        <div v-if="pumpRpm !== null" class="stat stat-pump">
          <span class="stat-label"><Droplet :size="15" />{{ t("devices.aio.statPump") }}</span>
          <span class="stat-value">{{ pumpRpm }}<small>rpm</small></span>
        </div>
        <div v-if="coolant !== null" class="stat">
          <span class="stat-label"><Thermometer :size="15" />{{ t("devices.aio.statCoolant") }}</span>
          <span class="stat-value">{{ coolant.toFixed(1) }}<small>°C</small></span>
          <span class="tone" :class="`tone-${coolantTone(coolant)}`" />
        </div>
        <div v-for="(rpm, i) in fanRpms" :key="'fan' + i" class="stat">
          <span class="stat-label"><Fan :size="15" />{{ t("devices.aio.fan", { index: i + 1 }) }}</span>
          <span class="stat-value">{{ rpm }}<small>rpm</small></span>
        </div>
      </div>
    </section>

    <section class="card panel">
      <h2 class="section-title">{{ t("devices.aio.coolingSection") }}</h2>

      <div class="slot">
        <div class="slot-id">
          <span class="slot-icon pump" :class="`kind-${kindOf(pumpMode)}`"><Droplet :size="20" /></span>
          <span class="slot-text">
            <span class="slot-label">{{ t("devices.aio.statPump") }}</span>
            <span class="mono slot-rpm">{{ pumpRpm !== null ? `${pumpRpm} rpm` : "—" }}</span>
          </span>
        </div>
        <SegmentedControl
          :model-value="kindOf(pumpMode)"
          :options="pumpKindOptions"
          :label="t('fans.groupCard.speedModeLabel', { slot: t('devices.aio.statPump') })"
          :disabled="!pumpControl"
          @update:model-value="onPumpKind"
        />
        <div class="slot-detail">
          <div v-if="pumpMode === 'constant'" class="fixed">
            <n-slider
              :value="pumpPwm()"
              :min="0"
              :max="255"
              :step="1"
              :format-tooltip="(v: number) => `${percentOf(v)}%`"
              :theme-overrides="fixedSliderTheme"
              :aria-label="t('fans.groupCard.fixedSpeedLabel', { slot: t('devices.aio.statPump') })"
              :disabled="!pumpControl"
              @update:value="(v: number) => setPumpPwm(Math.round(v))"
            />
            <NumberField
              class="fixed-field"
              :model-value="percentOf(pumpPwm())"
              :label="t('fans.groupCard.fixedSpeedLabel', { slot: t('devices.aio.statPump') })"
              unit="%"
              :disabled="!pumpControl"
              @update:model-value="(v) => setPumpPwm(pwmOf(v))"
            />
          </div>
          <div v-else-if="kindOf(pumpMode) === 'curve'" class="curve-chips">
            <button
              v-for="name in curveNames"
              :key="name"
              type="button"
              class="chip"
              :disabled="!pumpControl"
              :class="{ selected: pumpMode === `curve:${name}` }"
              :aria-pressed="pumpMode === `curve:${name}`"
              @click="onPumpMode(`curve:${name}`)"
            >
              {{ name }}
            </button>
          </div>
          <span v-else-if="pumpMode === '__mb_sync__'" class="note">
            {{ device.pump_mb_sync_support ? t("fans.groupCard.mbFollowing") : t("devices.aio.pumpMbSyncUnavailable") }}
          </span>
          <span v-else class="note">{{ t("fans.groupCard.deviceControlled") }}</span>
        </div>
        <span v-if="!pumpControl" class="note full">{{ t("devices.aio.controlUnavailable") }}</span>
      </div>

      <div v-for="slot in fanSlots" :key="slot - 1" class="slot">
        <div class="slot-id">
          <span class="slot-icon" :class="`kind-${kindOf(speedMode(aio.fan_speeds[slot - 1]))}`"><Fan :size="20" /></span>
          <span class="slot-text">
            <span class="slot-label">{{ t("devices.aio.fan", { index: slot }) }}</span>
            <span class="mono slot-rpm">{{ fanRpms[slot - 1] != null ? `${fanRpms[slot - 1]} rpm` : "—" }}</span>
          </span>
        </div>
        <SegmentedControl
          :model-value="kindOf(speedMode(aio.fan_speeds[slot - 1]))"
          :options="fanKindOptions"
          :label="t('fans.groupCard.speedModeLabel', { slot: t('devices.aio.fan', { index: slot }) })"
          :disabled="!hasFan"
          @update:model-value="(kind) => onFanKind(slot - 1, kind)"
        />
        <div class="slot-detail">
          <div v-if="speedMode(aio.fan_speeds[slot - 1]) === 'constant'" class="fixed">
            <n-slider
              :value="fanPwm(slot - 1)"
              :min="0"
              :max="255"
              :step="1"
              :format-tooltip="(v: number) => `${percentOf(v)}%`"
              :theme-overrides="fixedSliderTheme"
              :aria-label="t('fans.groupCard.fixedSpeedLabel', { slot: t('devices.aio.fan', { index: slot }) })"
              @update:value="(v: number) => setFanPwm(slot - 1, Math.round(v))"
            />
            <NumberField
              class="fixed-field"
              :model-value="percentOf(fanPwm(slot - 1))"
              :label="t('fans.groupCard.fixedSpeedLabel', { slot: t('devices.aio.fan', { index: slot }) })"
              unit="%"
              @update:model-value="(v) => setFanPwm(slot - 1, pwmOf(v))"
            />
          </div>
          <div v-else-if="kindOf(speedMode(aio.fan_speeds[slot - 1])) === 'curve'" class="curve-chips">
            <button
              v-for="name in curveNames"
              :key="name"
              type="button"
              class="chip"
              :class="{ selected: speedMode(aio.fan_speeds[slot - 1]) === `curve:${name}` }"
              :aria-pressed="speedMode(aio.fan_speeds[slot - 1]) === `curve:${name}`"
              @click="onFanMode(slot - 1, `curve:${name}`)"
            >
              {{ name }}
            </button>
          </div>
          <span v-else-if="speedMode(aio.fan_speeds[slot - 1]) === '__mb_sync__'" class="note">
            {{ device.mb_sync_support ? t("fans.groupCard.mbFollowing") : t("devices.aio.fanMbSyncUnavailable") }}
          </span>
          <span v-else class="note">{{ t("fans.groupCard.deviceControlled") }}</span>
        </div>
        <span v-if="!hasFan && slot === 1" class="note full">{{ t("devices.aio.controlUnavailable") }}</span>
      </div>
    </section>

    <section class="card panel">
      <h2 class="section-title">{{ t("devices.aio.screenSection") }}</h2>

      <div class="group">
        <h3 class="group-title">{{ t("devices.aio.readoutsGroup") }}</h3>
        <div class="grid">
          <div class="field"><label>{{ t("devices.aio.cpuTemp") }}</label>
            <SensorSelect size="small" :value="sensorValue('cpu_temp_source')" :options="sensorOptions" @update:value="(v) => onSensor('cpu_temp_source', v)" filterable />
          </div>
          <div class="field"><label>{{ t("devices.aio.cpuLoad") }}</label>
            <SensorSelect size="small" :value="sensorValue('cpu_load_source')" :options="sensorOptions" @update:value="(v) => onSensor('cpu_load_source', v)" filterable />
          </div>
          <div class="field"><label>{{ t("devices.aio.gpuTemp") }}</label>
            <SensorSelect size="small" :value="sensorValue('gpu_temp_source')" :options="sensorOptions" @update:value="(v) => onSensor('gpu_temp_source', v)" filterable />
          </div>
          <div class="field"><label>{{ t("devices.aio.gpuLoad") }}</label>
            <SensorSelect size="small" :value="sensorValue('gpu_load_source')" :options="sensorOptions" @update:value="(v) => onSensor('gpu_load_source', v)" filterable />
          </div>
        </div>
      </div>

      <div class="group">
        <h3 class="group-title">{{ t("devices.aio.lookGroup") }}</h3>
        <div class="grid">
          <div class="field">
            <label>{{ t("devices.aio.brightness") }}</label>
            <LabeledSlider :model-value="aio.brightness" :min="0" :max="100" suffix="%" @update:model-value="(v: number) => { aio.brightness = v; config.markDirty(); }" />
          </div>
          <div class="field">
            <label>{{ t("devices.aio.rotation") }}</label>
            <SegmentedControl
              size="small"
              :model-value="String(aio.rotation)"
              :options="rotationSegments"
              :label="t('devices.aio.rotation')"
              @update:model-value="onRotation"
            />
          </div>
          <div class="field">
            <label>{{ t("devices.aio.theme") }}</label>
            <n-select v-model:value="aio.theme_index" size="small" :options="themeOptions" @update:value="() => config.markDirty()" />
          </div>
          <div class="field">
            <label>{{ t("devices.aio.loopInterval") }}</label>
            <n-input-number v-model:value="aio.loop_interval" size="small" :min="1" :max="30" @update:value="() => config.markDirty()" />
          </div>
        </div>
      </div>

      <div class="group">
        <h3 class="group-title">{{ t("devices.aio.textColorsGroup") }}</h3>
        <div class="grid">
          <div class="field">
            <label>{{ t("devices.aio.labelColor") }}</label>
            <ColorPicker :model-value="aio.str_color" alpha @update:model-value="(v) => setColor('str_color', v)" />
          </div>
          <div class="field">
            <label>{{ t("devices.aio.valueColor") }}</label>
            <ColorPicker :model-value="aio.val_color" alpha @update:model-value="(v) => setColor('val_color', v)" />
          </div>
          <div class="field">
            <label>{{ t("devices.aio.unitColor") }}</label>
            <ColorPicker :model-value="aio.unit_color" alpha @update:model-value="(v) => setColor('unit_color', v)" />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.aio {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.hero {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
  padding: 28px 32px;
  border-radius: 32px;
  background: var(--md-tertiary-container);
  color: var(--md-on-tertiary-container);
}
.hero-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}
.hero-name {
  font-family: var(--font-display);
  font-size: 32px;
  font-weight: 600;
  line-height: 1.15;
}
.hero-id {
  font-size: var(--font-size-sm);
  opacity: 0.8;
}
.stats {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}
.stat {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 120px;
  padding: var(--space-4) 20px var(--space-4) var(--space-4);
  border-radius: 22px;
  background: var(--md-surface);
  color: var(--md-on-surface);
}
.stat-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.stat-value {
  font-family: var(--font-display);
  font-size: 30px;
  font-weight: 600;
  line-height: 1.05;
  font-variant-numeric: tabular-nums;
}
.stat-value small {
  margin-left: 3px;
  font-size: 14px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
}
.tone {
  position: absolute;
  top: var(--space-4);
  right: var(--space-4);
  width: 10px;
  height: 10px;
  border-radius: 50%;
}
.tone-cool { background: var(--temp-cool); }
.tone-warm { background: var(--temp-warm); }
.tone-hot { background: var(--temp-hot); }
.panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.panel .section-title {
  margin: 0 0 var(--space-1);
}
.slot {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3) 18px;
  padding: var(--space-3) var(--space-4) var(--space-3) var(--space-3);
  border-radius: 20px;
  background: var(--md-surface-container-high);
}
.slot-id {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 150px;
}
.slot-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  flex-shrink: 0;
  border-radius: 14px;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
}
.slot-icon.kind-curve {
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.slot-icon.kind-fixed {
  background: var(--md-tertiary-container);
  color: var(--md-on-tertiary-container);
}
.slot-icon.kind-mb {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.slot-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.slot-label {
  font-size: 14px;
  font-weight: 600;
}
.slot-rpm {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.slot-detail {
  flex: 1 1 260px;
  min-width: 0;
}
.fixed {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.fixed .n-slider {
  flex: 1;
  min-width: 80px;
}
.fixed-field {
  flex: 0 0 96px;
}
.curve-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.chip {
  height: 30px;
  max-width: 200px;
  padding: 0 var(--space-3);
  border: 1px solid var(--md-outline);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--md-on-surface-variant);
  font: inherit;
  font-size: var(--font-size-sm);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.chip:hover:not(.selected) {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.chip.selected {
  border-color: var(--md-primary-container);
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.chip:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 1px;
}
.note {
  font-size: 13px;
  color: var(--md-on-surface-variant);
}
.note.full {
  flex-basis: 100%;
}
.group {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: 18px 20px 20px;
  border-radius: 22px;
  background: var(--md-surface-container);
}
.group-title {
  margin: 0;
  font-size: var(--font-size-sm);
  font-weight: 700;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--space-3) var(--space-4);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}
.field > .segmented {
  align-self: flex-start;
}
.field > label {
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
</style>
