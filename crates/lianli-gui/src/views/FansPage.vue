<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { Plus, X } from "lucide-vue-next";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import FanCurveEditor from "@/components/fans/FanCurveEditor.vue";
import FanGroupCard from "@/components/fans/FanGroupCard.vue";
import SensorSelect from "@/components/common/SensorSelect.vue";
import type { SensorSource, DeviceInfo, FanGroup } from "@/types";
import { MB_SYNC_KEY } from "@/types";
import { enumerateSensorsAsOptions } from "@/stores/sensorOptions";
import { useTranslate, type MessageKey } from "@/i18n";
import { decodeSpeedMode, fillSlots, groupRunsCurves, isReservedCurveName, isSpeedModeKey, speedModeOf } from "@/components/fans/speedMode";

const config = useConfigStore();
const devices = useDevicesStore();
const names = useDeviceNamesStore();
const dialog = useDialog();
const message = useMessage();
const t = useTranslate();

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

const allFanModeOptions = computed(() => [
  { label: t("common.off"), value: "off" },
  ...curves.value.map((c) => ({ label: t("fans.speedMode.curve", { name: c.name }), value: `curve:${c.name}` })),
  { label: t("fans.speedMode.constantPwm"), value: "constant" },
  // Shown only to label uniform MB Sync; each device chooses its own sync source.
  { label: t("fans.speedMode.mbSync"), value: MB_SYNC_KEY, disabled: true },
]);

const allFansMode = computed(() => {
  const modes = new Set(
    assignmentGroups.value.flatMap(({ device, group }) =>
      Array.from({ length: Math.min(device.fan_count ?? 1, 4) }, (_, slot) => speedModeOf(group.speeds, slot)),
    ),
  );
  const [mode] = modes;
  return modes.size === 1 && allFanModeOptions.value.some((option) => option.value === mode) ? mode : null;
});

function applyToAllFans(value: string) {
  const option = allFanModeOptions.value.find((o) => o.value === value);
  if (!option || option.disabled) return;
  const groups = assignmentGroups.value.map(({ group }) => group);
  const speed = decodeSpeedMode(value);
  dialog.warning({
    title: t("fans.page.applyAllTitle"),
    content: t("fans.page.applyAllContent", { mode: option.label }),
    positiveText: t("common.apply"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => {
      for (const group of groups) fillSlots(group, speed);
      config.markDirty();
    },
  });
}

const fanLabels = computed(() => devices.labels);
function curveSlotCount(group: FanGroup, name: string): number {
  if (isSpeedModeKey(name) || !groupRunsCurves(group)) return 0;
  const device = group.device_id ? devices.byId(group.device_id) : undefined;
  const slots = device ? group.speeds.slice(0, device.fan_count ?? 1) : group.speeds;
  return slots.filter((speed) => speed === name).length;
}

function fanGroupUsers(name: string): string[] {
  return fanCfg.value.speeds.flatMap((group) => {
    const count = curveSlotCount(group, name);
    if (!count) return [];
    const device = group.device_id ? devices.byId(group.device_id) : undefined;
    if (!device) return [t("fans.page.curveUserOffline", { device: group.device_id ?? t("fans.page.unknownDevice") })];
    const label = names.label(device.device_id, fanLabels.value.get(device.device_id) ?? device.name);
    return [t(count === 1 ? "fans.page.curveUserOneFan" : "fans.page.curveUserFans", { label, count })];
  });
}

const curveUsers = computed(() => (current.value?.name && config.fanCurveRuns(current.value) ? fanGroupUsers(current.value.name) : []));

const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, true, true));

function onCurveChange(value: [number, number][]) {
  if (!current.value) return;
  current.value.curve = value;
  config.markDirty();
}

function addCurve() {
  const name = uniqueName(t("fans.page.newCurveName"), curves.value.map((c) => c.name));
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
  const curve = curves.value[idx];
  if (!curve) return;
  const name = curve.name || t("fans.page.curveFallbackName");
  dialog.error({
    title: t("fans.page.deleteCurveTitle"),
    content: t(config.fanCurveReferenced(curve) ? "fans.page.deleteCurveInUseContent" : "fans.page.deleteCurveContent", { name }),
    positiveText: t("common.delete"),
    negativeText: t("common.cancel"),
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

function curveNameError(name: string): MessageKey | null {
  if (!name) return "fans.page.curveNameEmpty";
  if (isReservedCurveName(name)) return "fans.page.curveNameReserved";
  if (curves.value.some((c) => c !== current.value && c.name === name)) return "fans.page.curveNameTaken";
  return null;
}

function isUnchangedName(input: string): boolean {
  const saved = current.value?.name;
  return input === saved || input.trim() === saved;
}

const nameError = computed(() => (isUnchangedName(nameBuffer.value) ? null : curveNameError(nameBuffer.value.trim())));

function renameCurve(input: string) {
  if (!current.value) return;
  if (isUnchangedName(input)) {
    nameBuffer.value = current.value.name;
    return;
  }
  const name = input.trim();
  const error = curveNameError(name);
  if (error) {
    message.warning(t(error, { name }));
    nameBuffer.value = current.value.name;
    return;
  }
  nameBuffer.value = name;
  config.renameFanCurve(current.value, name);
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
        <h2 class="section-title">{{ t("fans.page.fanCurves") }}</h2>
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
            :title="t('fans.page.deleteCurve')"
            @click.stop="removeCurve(i)"
          >
            <X :size="11" />
          </button>
        </div>
        <button class="curve-tab add" @click="addCurve">
          <Plus :size="13" /> {{ t("common.add") }}
        </button>
      </div>

      <div v-if="current" class="curve-body">
        <FanCurveEditor
          :model-value="current.curve"
          @update:model-value="onCurveChange"
        />
        <div class="curve-side">
          <div class="field">
            <label class="muted">{{ t("fans.page.name") }}</label>
            <n-input
              size="small"
              v-model:value="nameBuffer"
              :status="nameError ? 'error' : undefined"
              @blur="renameCurve(nameBuffer)"
              :placeholder="t('fans.page.curveNamePlaceholder')"
            />
          </div>
          <div class="field">
            <label class="muted">{{ t("fans.page.temperatureSource") }}</label>
            <SensorSelect
              size="small"
              :value="tempSourceValue"
              :options="sensorOptions"
              @update:value="onTempSource"
              filterable
            />
          </div>
          <div class="field" v-if="tempSourceValue === 'command' || tempSourceValue === ''">
            <label class="muted">{{ t("fans.page.customCommand") }}</label>
            <n-input
              size="small"
              v-model:value="commandBuffer"
              @blur="onTempCommand(commandBuffer)"
              :placeholder="t('fans.page.commandPlaceholder', { command: 'cat /sys/class/thermal/thermal_zone0/temp' })"
            />
          </div>
          <div class="field">
            <label class="muted">{{ t("fans.page.usedBy") }}</label>
            <span class="used-by">{{ curveUsers.length ? curveUsers.join(t("fans.page.usedBySeparator")) : t("fans.page.noFansUseCurve") }}</span>
          </div>
          <n-button
            v-if="assignmentGroups.length"
            size="small"
            class="use-for-all"
            @click="applyToAllFans(`curve:${current.name}`)"
          >
            {{ t("fans.page.useForAllFans") }}
          </n-button>
        </div>
      </div>
      <div v-else class="empty muted">{{ t("fans.page.noCurves") }}</div>
    </section>

    <!-- Section 2: Fan speed assignments -->
    <section class="card section">
      <div class="section-head">
        <h2 class="section-title">{{ t("fans.page.fanSpeedAssignments") }}</h2>
        <div v-if="assignmentGroups.length > 1" class="all-fans">
          <span class="muted">{{ t("fans.page.allFans") }}</span>
          <n-select
            size="small"
            :value="allFansMode"
            :options="allFanModeOptions"
            :placeholder="t('fans.groupCard.mixed')"
            @update:value="applyToAllFans"
          />
        </div>
      </div>
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
      <div v-else class="empty muted">{{ t("fans.page.noFanDevices") }}</div>
    </section>

    <section class="card settings-row">
      <h2 class="section-title">{{ t("fans.page.controller") }}</h2>
      <div class="field">
        <label class="muted">{{ t("fans.page.updateInterval") }}</label>
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
        <label class="muted" :title="t('fans.page.temperatureHysteresisHint')">{{ t("fans.page.temperatureHysteresis") }}</label>
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
        <label class="muted" :title="t('fans.page.speedHysteresisHint')">{{ t("fans.page.speedHysteresis") }}</label>
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
.use-for-all {
  align-self: flex-start;
}
.all-fans {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.all-fans .muted {
  white-space: nowrap;
}
.all-fans .n-select {
  width: 220px;
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
