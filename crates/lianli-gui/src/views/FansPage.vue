<script setup lang="ts">
import { computed, h, onMounted, onUnmounted, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { Download, Plus, Redo2, TriangleAlert, Undo2, Upload, X } from "lucide-vue-next";
import { invoke } from "@/utils/backendText";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useDaemonStore } from "@/stores/daemon";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import FanCurveEditor from "@/components/fans/FanCurveEditor.vue";
import CurvePointsEditor from "@/components/fans/CurvePointsEditor.vue";
import FanGroupCard from "@/components/fans/FanGroupCard.vue";
import SensorSelect from "@/components/common/SensorSelect.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import type { SensorSource, DeviceInfo } from "@/types";
import { MB_SYNC_KEY } from "@/types";
import { enumerateSensorsAsOptions } from "@/stores/sensorOptions";
import { useTranslate } from "@/i18n";
import { decodeSpeedMode, fillSlots, speedModeOf } from "@/components/fans/speedMode";
import { speedAt } from "@/components/fans/curvePoints";
import { CURVE_TEMPLATES } from "@/components/fans/curveTemplates";
import { useCurveHistory } from "@/components/fans/useCurveHistory";

const config = useConfigStore();
const devices = useDevicesStore();
const daemon = useDaemonStore();
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

type ModeKind = "curve" | "fixed" | "mb" | "off";

const allFansKind = computed<ModeKind | null>(() => {
  const mode = allFansMode.value;
  if (mode === null || mode === undefined) return null;
  if (mode === "constant") return "fixed";
  if (mode === "off") return "off";
  if (mode === MB_SYNC_KEY) return "mb";
  return "curve";
});

const allFansKindOptions = computed(() => [
  {
    value: "curve" as const,
    label: t("fans.groupCard.modeCurve"),
    disabled: !curves.value.length,
    title: current.value ? t("fans.live.allFansCurveTitle", { name: current.value.name }) : t("fans.groupCard.noCurves"),
  },
  { value: "fixed" as const, label: t("fans.groupCard.modeFixed") },
  { value: "mb" as const, label: t("fans.groupCard.modeMbSync"), disabled: true, title: t("fans.live.allFansMbTitle") },
  { value: "off" as const, label: t("common.off") },
]);

function onAllFansKind(kind: ModeKind) {
  if (kind === "mb") return;
  if (kind === "curve") {
    const name = current.value?.name ?? curves.value[0]?.name;
    if (name) applyToAllFans(`curve:${name}`);
    return;
  }
  applyToAllFans(kind === "fixed" ? "constant" : "off");
}

const lostCurves = computed(() => (daemon.connected ? devices.failingCurves : []));

function curveIndex(name: string): number {
  return curves.value.findIndex((curve) => curve.name === name);
}

const liveReading = computed(() => (current.value ? devices.curveTemperature(current.value.name) : null));

const liveState = computed(() => {
  const reading = liveReading.value;
  if (!reading || !current.value) return { kind: "idle", text: t("fans.live.notInUse"), tone: "" };
  if (reading.full_speed_fallback || reading.celsius === null) {
    return { kind: "fallback", text: t("fans.live.fallback"), tone: "" };
  }
  const celsius = reading.celsius;
  const speed = Math.round(speedAt(current.value.curve, celsius));
  const tone = celsius < 60 ? "cool" : celsius < 80 ? "warm" : "hot";
  return { kind: "reading", text: t("fans.live.marker", { temp: Math.round(celsius), speed }), tone };
});

const usesDefaultSource = computed(() => !current.value?.temp_source && !current.value?.temp_command);

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
const curveUsers = computed(() => {
  const name = current.value?.name;
  if (!name) return [];
  return fanCfg.value.speeds.flatMap((group) => {
    const device = group.device_id ? devices.byId(group.device_id) : undefined;
    const slots = device ? group.speeds.slice(0, device.fan_count ?? 1) : group.speeds;
    const count = slots.filter((speed) => speed === name).length;
    if (!count) return [];
    if (!device) return [t("fans.page.curveUserOffline", { device: group.device_id ?? t("fans.page.unknownDevice") })];
    const label = names.label(device.device_id, fanLabels.value.get(device.device_id) ?? device.name);
    return [t(count === 1 ? "fans.page.curveUserOneFan" : "fans.page.curveUserFans", { label, count })];
  });
});

const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, true, true));

const history = useCurveHistory(() => curves.value, () => config.markDirty());

function onCurveChange(value: [number, number][]) {
  if (!current.value) return;
  history.record(current.value, current.value.curve, value);
  current.value.curve = value;
  config.markDirty();
}

function isTextEntry(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));
}

function onHistoryKey(event: KeyboardEvent) {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || isTextEntry(event.target)) return;
  const key = event.key.toLowerCase();
  if (key === "z" && !event.shiftKey) {
    event.preventDefault();
    history.undo();
  } else if ((key === "z" && event.shiftKey) || key === "y") {
    event.preventDefault();
    history.redo();
  }
}

onMounted(() => window.addEventListener("keydown", onHistoryKey));
onUnmounted(() => window.removeEventListener("keydown", onHistoryKey));

interface PortableCurve {
  name: string;
  curve: [number, number][];
}

const fileBusy = ref(false);

async function exportCurves() {
  if (fileBusy.value || !curves.value.length) return;
  fileBusy.value = true;
  try {
    const saved = await invoke<boolean>("export_fan_curves", {
      curves: curves.value.map((c) => ({ name: c.name, curve: c.curve })),
      title: t("fans.curveFiles.exportTitle"),
      filter: t("fans.curveFiles.filter"),
    });
    if (saved) message.success(t("fans.curveFiles.exported"));
  } catch (error) {
    message.error(String(error));
  } finally {
    fileBusy.value = false;
  }
}

async function importCurves() {
  if (fileBusy.value) return;
  fileBusy.value = true;
  let found: PortableCurve[] = [];
  try {
    found = await invoke<PortableCurve[]>("import_fan_curves", {
      title: t("fans.curveFiles.importTitle"),
      filter: t("fans.curveFiles.filter"),
    });
  } catch (error) {
    message.error(String(error));
  } finally {
    fileBusy.value = false;
  }
  if (!found.length) return;
  const taken = curves.value.map((c) => c.name);
  const planned = found.map((curve) => {
    const name = uniqueName(curve.name, taken);
    taken.push(name);
    return { name, original: curve.name, curve: curve.curve };
  });
  dialog.info({
    title: t("fans.curveFiles.confirmTitle"),
    content: () =>
      h("div", { class: "import-preview" }, [
        h(
          "ul",
          planned.map((item) =>
            h("li", [
              h("strong", item.name),
              h("span", item.name === item.original
                ? t("fans.curveFiles.points", { count: item.curve.length })
                : t("fans.curveFiles.renamed", { name: item.original, count: item.curve.length })),
            ]),
          ),
        ),
        h("p", t("fans.curveFiles.sensorNote")),
      ]),
    positiveText: t("fans.curveFiles.importConfirm"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => {
      const first = curves.value.length;
      for (const item of planned) {
        config.addFanCurve({ name: item.name, temp_source: null, temp_command: "", curve: item.curve });
      }
      selectedCurve.value = first;
      message.success(t("fans.curveFiles.imported"));
    },
  });
}

const BLANK_CURVE = "blank";

const addCurveOptions = computed(() => [
  ...CURVE_TEMPLATES.map((template) => ({ key: template.id, label: t(template.nameKey) })),
  { type: "divider", key: "divider" },
  { key: BLANK_CURVE, label: t("fans.templates.blank") },
]);

function onAddCurve(key: string | number) {
  const template = CURVE_TEMPLATES.find((candidate) => candidate.id === key);
  if (template) {
    addCurveFrom(t(template.nameKey), template.points);
  } else if (key === BLANK_CURVE) {
    addCurveFrom(t("fans.page.newCurveName"), [[30, 30], [60, 70], [85, 100]]);
  }
}

function addCurveFrom(baseName: string, points: [number, number][]) {
  config.addFanCurve({
    name: uniqueName(baseName, curves.value.map((c) => c.name)),
    temp_source: null,
    temp_command: "",
    curve: points.map(([temp, speed]) => [temp, speed]),
  });
  selectedCurve.value = curves.value.length - 1;
}

function removeCurve(idx: number) {
  dialog.error({
    title: t("fans.page.deleteCurveTitle"),
    content: t("fans.page.deleteCurveContent", { name: curves.value[idx]?.name ?? t("fans.page.curveFallbackName") }),
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
    <div v-for="name in lostCurves" :key="name" class="sensor-lost" role="alert">
      <TriangleAlert :size="20" :stroke-width="2.2" aria-hidden="true" />
      <span class="sensor-lost-text">
        <strong>{{ t("fans.sensorLost.title", { name }) }}</strong>
        {{ t("fans.sensorLost.body") }}
      </span>
      <n-button v-if="curveIndex(name) >= 0" text type="primary" @click="selectedCurve = curveIndex(name)">
        {{ t("fans.sensorLost.check") }}
      </n-button>
    </div>

    <!-- Section 1: Fan curves -->
    <section class="card section">
      <div class="section-head">
        <h2 class="section-title">{{ t("fans.page.fanCurves") }}</h2>
        <div class="curve-actions">
          <n-button quaternary circle :disabled="!history.canUndo.value" :title="t('fans.history.undo')" :aria-label="t('fans.history.undo')" @click="history.undo()">
            <template #icon><Undo2 :size="18" /></template>
          </n-button>
          <n-button quaternary circle :disabled="!history.canRedo.value" :title="t('fans.history.redo')" :aria-label="t('fans.history.redo')" @click="history.redo()">
            <template #icon><Redo2 :size="18" /></template>
          </n-button>
          <span class="actions-divider" />
          <n-button round size="small" :disabled="fileBusy || !curves.length" @click="exportCurves">
            <template #icon><Download :size="15" /></template>
            {{ t("fans.curveFiles.export") }}
          </n-button>
          <n-button round size="small" :disabled="fileBusy" @click="importCurves">
            <template #icon><Upload :size="15" /></template>
            {{ t("fans.curveFiles.import") }}
          </n-button>
        </div>
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
          <span class="curve-tab-name" @click="selectedCurve = i">
            <span v-if="lostCurves.includes(c.name)" class="warn-dot" role="img" :aria-label="t('fans.sensorLost.chipTitle')" :title="t('fans.sensorLost.chipTitle')" />
            {{ c.name }}
          </span>
          <button
            class="curve-tab-del"
            :title="t('fans.page.deleteCurve')"
            @click.stop="removeCurve(i)"
          >
            <X :size="11" />
          </button>
        </div>
        <n-dropdown trigger="click" :options="addCurveOptions" @select="onAddCurve">
          <button class="curve-tab add" type="button" aria-haspopup="menu">
            <Plus :size="13" /> {{ t("common.add") }}
          </button>
        </n-dropdown>
      </div>

      <div v-if="current" class="curve-body">
        <div class="curve-main">
          <FanCurveEditor
            :model-value="current.curve"
            :live-celsius="liveReading?.celsius ?? null"
            @update:model-value="onCurveChange"
          />
          <CurvePointsEditor
            :model-value="current.curve"
            @update:model-value="onCurveChange"
          />
        </div>
        <div class="curve-side">
          <div class="now" :class="liveState.kind === 'reading' ? `tone-${liveState.tone}` : liveState.kind">
            <span class="now-label">{{ t("fans.live.rightNow") }}</span>
            <span v-if="liveState.kind === 'reading'" class="now-value">{{ liveState.text }}</span>
            <span v-else class="now-text">{{ liveState.text }}</span>
          </div>
          <div class="field">
            <label class="muted">{{ t("fans.page.name") }}</label>
            <n-input
              size="small"
              v-model:value="nameBuffer"
              @blur="renameCurve(nameBuffer)"
              :placeholder="t('fans.page.curveNamePlaceholder')"
            />
          </div>
          <div class="field">
            <label class="muted">{{ t("fans.page.temperatureSource") }}</label>
            <SensorSelect
              size="small"
              :value="tempSourceValue || null"
              :options="sensorOptions"
              :placeholder="t('fans.live.defaultSource')"
              @update:value="onTempSource"
              filterable
            />
            <span v-if="usesDefaultSource" class="source-hint">{{ t("fans.live.defaultSourceHint") }}</span>
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
          <span v-if="allFansKind === null" class="mixed">{{ t("fans.groupCard.mixed") }}</span>
          <SegmentedControl
            size="small"
            :model-value="allFansKind"
            :options="allFansKindOptions"
            :label="t('fans.page.allFans')"
            @update:model-value="onAllFansKind"
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
      <EmptyState v-else kind="fans" compact />
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
.curve-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-1) var(--space-2);
}
.actions-divider {
  width: 1px;
  height: 24px;
  background: var(--md-outline-variant);
}
:global(.import-preview ul) {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}
:global(.import-preview li) {
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
  padding: 10px 14px;
  border-radius: 14px;
  background: var(--md-surface-container-highest);
}
:global(.import-preview li span) {
  font-size: 12px;
  color: var(--md-on-surface-variant);
}
:global(.import-preview p) {
  margin: var(--space-3) 0 0;
  font-size: 13px;
  line-height: 1.45;
  color: var(--md-on-surface-variant);
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
  height: 34px;
  box-sizing: border-box;
  background: transparent;
  border: 1px solid var(--md-outline);
  color: var(--md-on-surface-variant);
  padding: 0 var(--space-2) 0 var(--space-4);
  border-radius: 10px;
  font-size: 14px;
  font-weight: 500;
  max-width: 220px;
  transition: background 0.12s, color 0.12s, border-color 0.12s;
}
.sensor-lost {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border-radius: 16px;
  background: color-mix(in srgb, var(--warning) 16%, var(--md-surface-container-low));
  color: var(--md-on-surface);
}
.sensor-lost > svg {
  flex-shrink: 0;
  color: var(--warning);
}
.sensor-lost-text {
  flex: 1;
  font-size: 13px;
  line-height: 1.45;
}
.warn-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  margin-right: 4px;
  border-radius: 50%;
  background: var(--warning);
  vertical-align: middle;
}
.curve-tab-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}
.curve-tab:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.curve-tab.active {
  background: var(--md-secondary-container);
  border-color: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
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
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
}
.curve-tab.add {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  border-style: dashed;
  padding: 0 var(--space-4);
  color: var(--md-primary);
  font: inherit;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
}
.curve-body {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(240px, 300px);
  gap: var(--space-6);
  align-items: start;
}
.curve-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  min-width: 0;
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
.all-fans {
  flex-wrap: wrap;
}
.mixed {
  padding: 3px 10px;
  border-radius: 10px;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-size: var(--font-size-xs);
  font-weight: 600;
}
.now {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--space-3) var(--space-4);
  border-radius: 18px;
  background: var(--md-surface-container-high);
}
.now-label {
  font-size: var(--font-size-xs);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.now-value {
  font-family: var(--font-display);
  font-size: 26px;
  font-weight: 600;
  line-height: 1.2;
}
.now-text {
  font-size: 13px;
  color: var(--md-on-surface-variant);
}
.now.tone-cool .now-value { color: var(--temp-cool); }
.now.tone-warm .now-value { color: var(--temp-warm); }
.now.tone-hot .now-value { color: var(--temp-hot); }
.now.fallback {
  background: color-mix(in srgb, var(--danger) 16%, var(--md-surface-container-high));
}
.now.fallback .now-text {
  color: var(--md-on-surface);
}
.source-hint {
  font-size: var(--font-size-xs);
  line-height: 1.45;
  color: var(--md-on-surface-variant);
}
.used-by {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
@media (max-width: 1080px) {
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
  gap: var(--space-3);
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
