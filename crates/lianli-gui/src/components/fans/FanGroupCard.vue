<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, ChevronRight, Fan, Info } from "lucide-vue-next";
import type { DeviceInfo, FanGroup, FanSpeed, PwmHeader } from "@/types";
import { MB_SYNC_KEY, MB_SYNC_PREFIX } from "@/types";
import { decodeSpeedMode, fillSlots, isMbSyncGroup, speedModeOf } from "./speedMode";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useDaemonStore } from "@/stores/daemon";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { useThemeStore } from "@/stores/theme";
import NumberField from "@/components/common/NumberField.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import { FAMILY_DISPLAY } from "@/constants";
import { useTranslate } from "@/i18n";

const props = defineProps<{
  device: DeviceInfo;
  group: FanGroup;
  curveNames: string[];
  pwmHeaders: PwmHeader[];
}>();

const config = useConfigStore();
const t = useTranslate();

const expanded = ref(true);

const devices = useDevicesStore();
const daemon = useDaemonStore();
const names = useDeviceNamesStore();

const title = computed(() =>
  names.label(props.device.device_id, devices.labels.get(props.device.device_id) ?? props.device.name),
);
const family = computed(() => {
  const fam = FAMILY_DISPLAY[props.device.family] ?? props.device.family;
  return fam === props.device.name ? null : fam;
});

const rpms = computed(() => devices.fanRpms(props.device.device_id));
const visibleSlots = computed(() => Math.min(slotCount.value, 4));

const commonMode = computed(() => {
  const first = modeOf(0);
  for (let slot = 1; slot < visibleSlots.value; slot++) {
    if (modeOf(slot) !== first) return null;
  }
  return first;
});

function onAllModes(value: string) {
  if (value === MB_SYNC_KEY) {
    onMode(0, value);
    return;
  }
  setAllSlots(decodeSpeedMode(value));
}

const perFan = computed(() => props.device.per_fan_control ?? false);
const slotCount = computed(() => props.device.fan_count ?? 1);

function speedAt(slot: number): FanSpeed {
  return props.group.speeds[slot];
}

function setSpeed(slot: number, value: FanSpeed) {
  const next = [...props.group.speeds] as FanSpeed[];
  next[slot] = value;
  props.group.speeds = next as [FanSpeed, FanSpeed, FanSpeed, FanSpeed];
  config.markDirty();
}

// MB Sync is a port-wide hardware setting, not per-fan: selecting it applies
// to all slots, and leaving it clears it from the whole port.
function groupIsMbSync(): boolean {
  return isMbSyncGroup(props.group.speeds);
}
function setAllSlots(value: FanSpeed) {
  fillSlots(props.group, value);
  config.markDirty();
}

// PWM header options for the MB Sync source dropdown.
const pwmHeaderOptions = computed(() => {
  if (!props.pwmHeaders.length) return [];
  return props.pwmHeaders.map((h) => ({ label: h.label, value: h.id }));
});

function modeOf(slot: number): string {
  return speedModeOf(props.group.speeds, slot);
}

function onMode(slot: number, value: string) {
  if (value === MB_SYNC_KEY) {
    if (props.device.mb_sync_support) {
      setAllSlots(MB_SYNC_KEY);
    } else {
      const source = currentPwmSource.value || props.pwmHeaders[0]?.id;
      if (source) setAllSlots(`${MB_SYNC_PREFIX}${source}`);
    }
    return;
  }
  const decoded = decodeSpeedMode(value);
  if (groupIsMbSync()) {
    // Leaving MB Sync is also port-wide: reset the whole port to a constant
    // default, then apply the chosen mode to this fan.
    const next: FanSpeed[] = [128, 128, 128, 128];
    next[slot] = decoded;
    props.group.speeds = next as [FanSpeed, FanSpeed, FanSpeed, FanSpeed];
    config.markDirty();
  } else {
    setSpeed(slot, decoded);
  }
}

// ── PWM source selection (MB Sync) ──────────────────────────────────────────
// Returns the header id from the first slot that has one, or empty string.
const currentPwmSource = computed(() => {
  for (const s of props.group.speeds) {
    if (typeof s === "string" && s.startsWith(MB_SYNC_PREFIX)) {
      return s.slice(MB_SYNC_PREFIX.length);
    }
  }
  return "";
});

function onPwmSource(headerId: string | null) {
  if (!headerId) return;
  const value = `${MB_SYNC_PREFIX}${headerId}`;
  setAllSlots(value);
}

function pwmOf(slot: number): number {
  const s = speedAt(slot);
  return typeof s === "number" ? s : 128;
}
function setPwm(slot: number, v: number) {
  setSpeed(slot, v);
}

function pctOf(slot: number): number {
  return Math.round((pwmOf(slot) / 255) * 100);
}
function setPct(slot: number, pct: number) {
  setPwm(slot, Math.round((pct / 100) * 255));
}

function slotLabel(slot: number): string {
  return t(perFan.value ? "fans.groupCard.fanSlot" : "fans.groupCard.portSlot", { index: slot });
}

type ModeKind = "curve" | "fixed" | "mb" | "off";

function kindOf(mode: string | null): ModeKind | null {
  if (mode === null) return null;
  if (mode === MB_SYNC_KEY) return "mb";
  if (mode === "constant") return "fixed";
  if (mode === "off") return "off";
  return "curve";
}

const commonKind = computed(() => kindOf(commonMode.value));

const theme = useThemeStore();
const fixedSliderTheme = computed(() => ({
  fillColor: theme.palette.tertiary,
  fillColorHover: theme.palette.tertiary,
}));

const kindOptions = computed(() => [
  {
    value: "curve" as const,
    label: t("fans.groupCard.modeCurve"),
    disabled: !props.curveNames.length,
    title: props.curveNames.length ? undefined : t("fans.groupCard.curveUnavailable"),
  },
  { value: "fixed" as const, label: t("fans.groupCard.modeFixed") },
  {
    value: "mb" as const,
    label: t("fans.groupCard.modeMbSync"),
    disabled: !mbAvailable.value,
    title: mbAvailable.value ? undefined : t("fans.groupCard.mbUnavailable"),
  },
  { value: "off" as const, label: t("common.off") },
]);

const mbAvailable = computed(() => props.device.mb_sync_support || props.pwmHeaders.length > 0);

const failingCurves = computed(() => new Set(daemon.connected ? devices.failingCurves : []));

function slotCurveFailing(slot: number): boolean {
  const mode = modeOf(slot);
  return mode.startsWith("curve:") && failingCurves.value.has(mode.slice("curve:".length));
}

function kindValue(kind: ModeKind, slot: number): string | null {
  if (kind === "curve") {
    const current = modeOf(slot);
    if (current.startsWith("curve:") && props.curveNames.includes(current.slice("curve:".length))) return current;
    return props.curveNames.length ? `curve:${props.curveNames[0]}` : null;
  }
  if (kind === "fixed") return "constant";
  if (kind === "mb") return MB_SYNC_KEY;
  return "off";
}

function onKind(slot: number, kind: ModeKind) {
  if (kindOf(modeOf(slot)) === kind) return;
  const value = kindValue(kind, slot);
  if (value) onMode(slot, value);
}

function onAllKind(kind: ModeKind) {
  const value = kindValue(kind, 0);
  if (value) onAllModes(value);
}
</script>

<template>
  <div class="fan-group">
    <div class="head">
      <button class="toggle" :aria-expanded="expanded" @click="expanded = !expanded">
        <component :is="expanded ? ChevronDown : ChevronRight" :size="18" />
        <span class="heading">
          <span class="title">{{ title }}</span>
          <span class="meta">
            <template v-if="family">{{ family }} · </template>{{ t(perFan ? "fans.groupCard.fanCount" : "fans.groupCard.portCount", { count: visibleSlots }) }}
          </span>
        </span>
      </button>
      <div v-if="visibleSlots > 1" class="all">
        <span class="all-label">{{ t("fans.groupCard.all") }}</span>
        <SegmentedControl
          size="small"
          :model-value="commonKind"
          :options="kindOptions"
          :label="t('fans.groupCard.allModesLabel', { device: title })"
          @update:model-value="onAllKind"
        />
      </div>
    </div>

    <p v-if="!mbAvailable" class="unavailable-note">
      <Info :size="16" :stroke-width="2.2" aria-hidden="true" />
      <span>{{ t("fans.groupCard.mbUnavailable") }}</span>
    </p>

    <div v-if="expanded" class="slots">
      <div v-for="slot in visibleSlots" :key="slot - 1" class="slot">
        <div class="slot-id">
          <span class="slot-icon" :class="slotCurveFailing(slot - 1) ? 'kind-failing' : `kind-${kindOf(modeOf(slot - 1))}`"><Fan :size="20" /></span>
          <span class="slot-text">
            <span class="slot-label">{{ slotLabel(slot) }}</span>
            <span class="mono rpm" :title="t('fans.groupCard.fanSpeedTooltip', { index: slot })">
              {{ rpms[slot - 1] != null ? `${rpms[slot - 1]} rpm` : "—" }}
            </span>
          </span>
        </div>
        <SegmentedControl
          :model-value="kindOf(modeOf(slot - 1))"
          :options="kindOptions"
          :label="t('fans.groupCard.speedModeLabel', { slot: slotLabel(slot) })"
          @update:model-value="(kind) => onKind(slot - 1, kind)"
        />
        <div class="slot-detail">
          <div v-if="modeOf(slot - 1) === 'constant'" class="fixed">
            <n-slider
              :value="pwmOf(slot - 1)"
              :min="0"
              :max="255"
              :step="1"
              :format-tooltip="(v: number) => `${Math.round((v / 255) * 100)}%`"
              :theme-overrides="fixedSliderTheme"
              :aria-label="t('fans.groupCard.fixedSpeedLabel', { slot: slotLabel(slot) })"
              @update:value="(v: number) => setPwm(slot - 1, Math.round(v))"
            />
            <NumberField
              class="fixed-field"
              :model-value="pctOf(slot - 1)"
              :label="t('fans.groupCard.fixedSpeedLabel', { slot: slotLabel(slot) })"
              unit="%"
              @update:model-value="(v) => setPct(slot - 1, v)"
            />
          </div>
          <div v-else-if="kindOf(modeOf(slot - 1)) === 'curve'" class="curve-chips">
            <button
              v-for="name in curveNames"
              :key="name"
              type="button"
              class="chip"
              :class="{ selected: modeOf(slot - 1) === `curve:${name}` }"
              :aria-pressed="modeOf(slot - 1) === `curve:${name}`"
              :title="failingCurves.has(name) ? t('fans.sensorLost.chipTitle') : undefined"
              @click="onMode(slot - 1, `curve:${name}`)"
            >
              <span v-if="failingCurves.has(name)" class="warn-dot" aria-hidden="true" />
              {{ name }}
            </button>
            <span v-if="slotCurveFailing(slot - 1)" class="failing-text">{{ t("fans.sensorLost.slot") }}</span>
          </div>
          <span v-else-if="modeOf(slot - 1) === MB_SYNC_KEY" class="detail-text">
            {{ device.mb_sync_support ? t("fans.groupCard.mbFollowing") : t("fans.groupCard.mbFollowingSource") }}
          </span>
          <span v-else class="detail-text">{{ t("fans.groupCard.deviceControlled") }}</span>
        </div>
      </div>
      <p v-if="groupIsMbSync() && visibleSlots > 1" class="port-wide">{{ t("fans.groupCard.mbPortWide") }}</p>
    </div>

    <div v-if="groupIsMbSync() && !device.mb_sync_support" class="pwm-source-row">
      <label class="pwm-label">{{ t("fans.groupCard.pwmSource") }}</label>
      <n-select
        size="small"
        :value="currentPwmSource"
        :options="pwmHeaderOptions"
        :placeholder="t('fans.groupCard.pwmSourcePlaceholder')"
        @update:value="onPwmSource"
      />
      <span class="hint">{{ t("fans.groupCard.pwmSourceHint") }}</span>
    </div>
  </div>
</template>

<style scoped>
.fan-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-4) var(--space-4) var(--space-3);
  border-radius: 24px;
  border: 1px solid var(--md-outline-variant);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-4);
}
.toggle {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  padding: var(--space-1);
  border: none;
  border-radius: var(--radius-md);
  background: none;
  color: var(--md-on-surface);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.toggle:focus-visible {
  outline: 2px solid var(--md-primary);
}
.heading {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.title {
  font-size: 16px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.meta {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.all {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.all-label {
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.slots {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
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
.slot-icon.kind-failing {
  background: color-mix(in srgb, var(--warning) 18%, var(--md-surface-container-high));
  color: var(--warning);
}
.unavailable-note {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border-radius: 14px;
  background: var(--md-surface-container-high);
  font-size: 13px;
  line-height: 1.45;
  color: var(--md-on-surface-variant);
}
.unavailable-note svg {
  flex-shrink: 0;
  margin-top: 1px;
  color: var(--md-primary);
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
.rpm {
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
.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.warn-dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--warning);
}
.failing-text {
  align-self: center;
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--warning);
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
.detail-text {
  font-size: 13px;
  color: var(--md-on-surface-variant);
}
.port-wide {
  margin: 0;
  padding-left: var(--space-1);
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.pwm-source-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
  padding-left: var(--space-1);
}
.pwm-label {
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.pwm-source-row .n-select {
  width: 260px;
}
</style>
