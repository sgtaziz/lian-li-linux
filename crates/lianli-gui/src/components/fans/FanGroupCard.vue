<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, ChevronRight } from "lucide-vue-next";
import type { DeviceInfo, FanGroup, FanSpeed, PwmHeader } from "@/types";
import { MB_SYNC_KEY, MB_SYNC_PREFIX } from "@/types";
import { useConfigStore } from "@/stores/config";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { FAMILY_DISPLAY } from "@/constants";

const props = defineProps<{
  device: DeviceInfo;
  group: FanGroup;
  curveNames: string[];
  pwmHeaders: PwmHeader[];
}>();

const config = useConfigStore();

const expanded = ref(true);

const title = computed(() => {
  const fam = FAMILY_DISPLAY[props.device.family] ?? props.device.family;
  return `${fam} (${props.device.name})`;
});

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

// MB Sync is a port-wide hardware setting, not per-fan: when it's on, every
// fan on the port follows the motherboard PWM header. So selecting it applies
// to all slots, and leaving it clears it from the whole port.
function groupIsMbSync(): boolean {
  return props.group.speeds.some(
    (s) => typeof s === "string" && s.startsWith("__mb_sync__"),
  );
}
function setAllSlots(value: FanSpeed) {
  props.group.speeds = [value, value, value, value] as [
    FanSpeed,
    FanSpeed,
    FanSpeed,
    FanSpeed,
  ];
  config.markDirty();
}
function decodeMode(value: string): FanSpeed {
  if (value === "off") return "off";
  if (value === "constant") return 128;
  if (value.startsWith("curve:")) return value.slice("curve:".length);
  return 128;
}

// Speed mode dropdown options: Off / curve names / Constant PWM / MB Sync.
const modeOptions = computed(() => {
  const opts: { label: string; value: string }[] = [
    { label: "Off", value: "off" },
    ...props.curveNames.map((n) => ({ label: `Curve: ${n}`, value: `curve:${n}` })),
    { label: "Constant PWM", value: "constant" },
    { label: "MB Sync", value: "__mb_sync__" },
  ];
  return opts;
});

// PWM header options for the MB Sync source dropdown.
const pwmHeaderOptions = computed(() => {
  if (!props.pwmHeaders.length) return [];
  return props.pwmHeaders.map((h) => ({ label: h.label, value: h.id }));
});

function modeOf(slot: number): string {
  // MB Sync is port-wide: if any slot is MB Sync, every fan on the port is —
  // even when the stored config only marks one slot (e.g. loaded from disk or
  // authored by the Slint GUI, which wrote it per-slot).
  if (groupIsMbSync()) return "__mb_sync__";
  const s = speedAt(slot);
  if (typeof s === "number") return "constant";
  if (s === "off" || s === "") return "off";
  return `curve:${s}`;
}

function onMode(slot: number, value: string) {
  if (value === "__mb_sync__") {
    // Port-wide: every fan on this port becomes MB Sync.
    // Preserve any existing PWM source, default to bare __mb_sync__.
    setAllSlots(MB_SYNC_KEY);
    return;
  }
  const decoded = decodeMode(value);
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
</script>

<template>
  <div class="card fan-group">
    <div class="head" @click="expanded = !expanded">
      <component :is="expanded ? ChevronDown : ChevronRight" :size="16" />
      <span class="title">{{ title }}</span>
      <span class="muted">{{ perFan ? "per-fan control" : "per-port control" }}</span>
    </div>

    <div v-if="expanded" class="slots">
      <div
        v-for="slot in Math.min(slotCount, 4)"
        :key="slot - 1"
        class="slot"
      >
        <div class="slot-label">Fan {{ perFan ? slot : `Port ${slot}` }}</div>
        <n-select
          size="small"
          :value="modeOf(slot - 1)"
          :options="modeOptions"
          @update:value="(v: string) => onMode(slot - 1, v)"
        />
        <div v-if="modeOf(slot - 1) === 'constant'" class="pwm">
          <LabeledSlider
            :model-value="pwmOf(slot - 1)"
            :min="0"
            :max="255"
            :step="1"
            suffix="%"
            @update:model-value="(v: number) => setPwm(slot - 1, Math.round((v / 255) * 255))"
          />
          <span class="pwm-pct">{{ Math.round((pwmOf(slot - 1) / 255) * 100) }}%</span>
        </div>
      </div>
    </div>

    <!-- PWM source picker: only for devices without hardware MB sync (e.g. wireless) -->
    <div v-if="groupIsMbSync() && !device.mb_sync_support && pwmHeaderOptions.length" class="pwm-source-row">
      <label class="muted">PWM source</label>
      <n-select
        size="small"
        :value="currentPwmSource"
        :options="pwmHeaderOptions"
        placeholder="Select motherboard PWM header"
        @update:value="onPwmSource"
      />
    </div>
  </div>
</template>

<style scoped>
.fan-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.head {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
}
.head .muted {
  margin-left: auto;
}
.title {
  font-weight: 600;
}
.slots {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--space-3);
}
.slot {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.slot-label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
.pwm {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.pwm-pct {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  text-align: right;
}
.pwm-source-row {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
</style>
