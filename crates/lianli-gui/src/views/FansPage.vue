<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { Plus, X } from "lucide-vue-next";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import FanCurveEditor from "@/components/fans/FanCurveEditor.vue";
import FanGroupCard from "@/components/fans/FanGroupCard.vue";
import SensorSelect from "@/components/common/SensorSelect.vue";
import type { SensorSource, DeviceInfo } from "@/types";
import { enumerateSensorsAsOptions } from "@/stores/sensorOptions";

const config = useConfigStore();
const devices = useDevicesStore();
const dialog = useDialog();

const selectedCurve = ref(0);

const curves = computed(() => config.config.fan_curves);
const current = computed(() => curves.value[selectedCurve.value] ?? null);

// Local edit buffers for the name/command text inputs. Binding these fields
// directly to `current.name`/`current.temp_command` via a plain `:value`
// (rather than `v-model`) means Vue force-writes the DOM `value` on every
// re-render of this page — which happens every ~2s from the telemetry
// polling loop — silently discarding any keystrokes typed since the last
// tick. Buffering locally and only resyncing when the *selected curve*
// changes (not on every render) avoids that.
const nameBuffer = ref(current.value?.name ?? "");
const commandBuffer = ref(current.value?.temp_command ?? "");
watch(current, (c) => {
  nameBuffer.value = c?.name ?? "";
  commandBuffer.value = c?.temp_command ?? "";
});

const fanCfg = computed(() => config.ensureFans());

// Fan groups matched to non-AIO fan devices.
const assignmentGroups = computed(() => {
  const result: { device: DeviceInfo; group: any }[] = [];
  for (const dev of devices.fanDevices) {
    // Skip AIOs (handled on the AIO page).
    const fam = dev.family;
    if (
      fam === "Galahad2Trinity" ||
      fam === "HydroShiftLcd" ||
      fam === "Galahad2Lcd" ||
      fam === "HydroShift2Lcd" ||
      fam === "HydroShift2OledCurveLed" ||
      fam === "WirelessAio"
    ) {
      continue;
    }
    let group = fanCfg.value.speeds.find((g) => g.device_id === dev.device_id);
    if (!group) {
      group = { device_id: dev.device_id, speeds: new Array(dev.fan_count ?? 1).fill(128) };
      fanCfg.value.speeds.push(group);
    }
    result.push({ device: dev, group });
  }
  return result;
});

const curveUsers = computed(() => {
  const name = current.value?.name;
  if (!name) return [];
  return fanCfg.value.speeds.flatMap((group) => {
    const device = group.device_id ? devices.byId(group.device_id) : undefined;
    const slots = device ? group.speeds.slice(0, device.fan_count ?? 1) : group.speeds;
    const count = slots.filter((speed) => speed === name).length;
    if (!count) return [];
    if (!device) return [`${group.device_id ?? "Unknown device"} (offline)`];
    return [`${device.name} (${count} fan${count === 1 ? "" : "s"})`];
  });
});

const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, true, true));

function onCurveChange(value: [number, number][]) {
  if (!current.value) return;
  current.value.curve = value;
  config.markDirty();
}

function addCurve() {
  const name = uniqueName("New Curve", curves.value.map((c) => c.name));
  config.addFanCurve({
    name,
    temp_source: null,
    temp_command: "",
    curve: [
      [30, 30],
      [60, 70],
      [85, 100],
    ],
  });
  selectedCurve.value = curves.value.length - 1;
}

function removeCurve(idx: number) {
  dialog.error({
    title: "Delete curve?",
    content: `"${curves.value[idx]?.name ?? "Curve"}" will be permanently deleted.`,
    positiveText: "Delete",
    negativeText: "Cancel",
    onPositiveClick: () => {
      curves.value.splice(idx, 1);
      if (selectedCurve.value > idx) selectedCurve.value -= 1;
      if (selectedCurve.value >= curves.value.length) {
        selectedCurve.value = Math.max(0, curves.value.length - 1);
      }
      config.markDirty();
    },
  });
}

function renameCurve(name: string) {
  if (!current.value) return;
  current.value.name = name;
  config.markDirty();
}

function onTempSource(value: string) {
  if (!current.value) return;
  if (value === "command") {
    current.value.temp_source = { type: "command", cmd: current.value.temp_command ?? "" };
  } else {
    current.value.temp_source = decodeSensorOption(value);
  }
  config.markDirty();
}

function onTempCommand(value: string) {
  if (!current.value) return;
  current.value.temp_command = value;
  if (current.value.temp_source && current.value.temp_source.type === "command") {
    current.value.temp_source.cmd = value;
  }
  config.markDirty();
}

const tempSourceValue = computed(() => {
  if (!current.value?.temp_source) return "";
  const s = current.value.temp_source;
  if (s.type === "command") return current.value.temp_command ? "command" : "";
  return encodeSensorOption(s);
});

function uniqueName(base: string, taken: string[]): string {
  if (!taken.includes(base)) return base;
  for (let i = 2; i < 1000; i++) {
    const c = `${base} ${i}`;
    if (!taken.includes(c)) return c;
  }
  return base;
}

function encodeSensorOption(s: SensorSource): string {
  return JSON.stringify(s);
}
function decodeSensorOption(v: string): SensorSource {
  try {
    return JSON.parse(v) as SensorSource;
  } catch {
    return { type: "command", cmd: "" };
  }
}

// ── Fan controller settings ────────────────────────────────────────────────
function onUpdateInterval(v: number | null) {
  if (v === null) return;
  fanCfg.value.update_interval_ms = v;
  config.markDirty();
}
function onHysteresisTemp(v: number | null) {
  if (v === null) return;
  fanCfg.value.hysteresis_temp = Math.round(v * 10) / 10;
  config.markDirty();
}
function onHysteresisPwm(percent: number | null) {
  if (percent === null) return;
  fanCfg.value.hysteresis_pwm = Math.min(50, Math.round((percent / 100) * 255));
  config.markDirty();
}
</script>

<template>
  <div class="page fans-page">
    <!-- Section 1: Fan curves -->
    <section class="card section">
      <div class="section-head">
        <h2 class="section-title">Fan Curves</h2>
      </div>

      <!-- Curve selector as tabs (clearer than a dropdown + separate label).
           Each tab carries an inline delete icon. -->
      <div class="curve-tabs">
        <div
          v-for="(c, i) in curves"
          :key="i"
          class="curve-tab"
          :class="{ active: i === selectedCurve }"
          :title="c.name"
        >
          <span class="curve-tab-name" @click="selectedCurve = i">{{ c.name }}</span>
          <button
            class="curve-tab-del"
            title="Delete curve"
            @click.stop="removeCurve(i)"
          >
            <X :size="11" />
          </button>
        </div>
        <button class="curve-tab add" @click="addCurve">
          <Plus :size="13" /> Add
        </button>
      </div>

      <div v-if="current" class="curve-body">
        <FanCurveEditor
          :model-value="current.curve"
          @update:model-value="onCurveChange"
        />
        <div class="curve-side">
          <div class="field">
            <label class="muted">Name</label>
            <n-input
              size="small"
              v-model:value="nameBuffer"
              @blur="renameCurve(nameBuffer)"
              placeholder="Curve name"
            />
          </div>
          <div class="field">
            <label class="muted">Temperature source</label>
            <SensorSelect
              size="small"
              :value="tempSourceValue"
              :options="sensorOptions"
              @update:value="onTempSource"
              filterable
            />
          </div>
          <div class="field" v-if="tempSourceValue === 'command' || tempSourceValue === ''">
            <label class="muted">Custom command</label>
            <n-input
              size="small"
              v-model:value="commandBuffer"
              @blur="onTempCommand(commandBuffer)"
              placeholder="e.g. cat /sys/class/thermal/thermal_zone0/temp"
            />
          </div>
          <div class="field">
            <label class="muted">Used by</label>
            <span class="used-by">{{ curveUsers.length ? curveUsers.join(", ") : "No fans use this curve" }}</span>
          </div>
        </div>
      </div>
      <div v-else class="empty muted">No fan curves. Click "Add" to create one.</div>
    </section>

    <!-- Section 2: Fan speed assignments -->
    <section class="card section">
      <h2 class="section-title">Fan Speed Assignments</h2>
      <div v-if="assignmentGroups.length" class="groups">
        <FanGroupCard
          v-for="ag in assignmentGroups"
          :key="ag.device.device_id"
          :device="ag.device"
          :group="ag.group"
          :curve-names="curves.map((c) => c.name)"
          :pwm-headers="config.pwmHeaders"
        />
      </div>
      <div v-else class="empty muted">No fan devices detected.</div>
    </section>

    <section class="card settings-row">
      <h2 class="section-title">Controller</h2>
      <div class="field">
        <label class="muted">Update interval</label>
        <n-input-number
          size="small"
          :value="fanCfg.update_interval_ms"
          :min="100"
          :max="10000"
          :step="100"
          @update:value="onUpdateInterval"
        ><template #suffix>ms</template></n-input-number>
      </div>
      <div class="field">
        <label class="muted" title="Ignore temperature changes smaller than this">Temperature hysteresis</label>
        <n-input-number
          size="small"
          :value="fanCfg.hysteresis_temp"
          :min="0"
          :max="10"
          :step="0.1"
          :precision="1"
          @update:value="onHysteresisTemp"
        ><template #suffix>°C</template></n-input-number>
      </div>
      <div class="field">
        <label class="muted" title="Ignore speed changes smaller than this">Speed hysteresis</label>
        <n-input-number
          size="small"
          :value="Math.round((fanCfg.hysteresis_pwm / 255) * 1000) / 10"
          :min="0"
          :max="19.6"
          :step="0.4"
          :precision="1"
          @update:value="onHysteresisPwm"
        ><template #suffix>%</template></n-input-number>
      </div>
    </section>
  </div>
</template>

<style scoped>
.fans-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 1100px;
}
.section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.section-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.curve-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.curve-tab {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  padding: var(--space-1) var(--space-2) var(--space-1) var(--space-3);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  max-width: 200px;
  transition: background 0.12s, color 0.12s, border-color 0.12s;
}
.curve-tab-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}
.curve-tab:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}
.curve-tab.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent);
}
.curve-tab-del {
  display: inline-flex;
  align-items: center;
  background: transparent;
  border: none;
  color: inherit;
  opacity: 0.55;
  cursor: pointer;
  padding: 1px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
}
.curve-tab-del:hover {
  opacity: 1;
  background: rgba(248, 113, 113, 0.18);
  color: var(--danger);
}
.curve-tab.add {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  border-style: dashed;
  padding: var(--space-1) var(--space-3);
  cursor: pointer;
}
.curve-body {
  display: grid;
  grid-template-columns: auto minmax(220px, 1fr);
  gap: var(--space-4);
  align-items: start;
}
.curve-side {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}
.used-by {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
@media (max-width: 860px) {
  .curve-body {
    grid-template-columns: 1fr;
  }
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.groups {
  display: flex;
  flex-direction: column;
  gap: 0;
}
.settings-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-2) var(--space-4);
}
.settings-row .section-title {
  margin: 0 var(--space-2) 6px 0;
}
.settings-row .field {
  width: 150px;
}
.empty {
  padding: var(--space-4);
}
</style>
