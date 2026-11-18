<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { FolderOpen, Plus, Trash2 } from "lucide-vue-next";
import type { Widget, RGBA, RGB, SensorSourceConfig, SensorRange, GradientStop } from "@/types";
import { pickMediaFile } from "@/utils/mediaPicker";
import SensorSelect from "@/components/common/SensorSelect.vue";
import { SENSOR_CATEGORY_LABELS } from "@/stores/sensorOptions";
import { useTranslate, type MessageKey } from "@/i18n";

const props = defineProps<{
  widget: Widget | null;
  sensorOptions: { label: string; value: string }[];
  fontOptions: { label: string; value: string }[];
}>();

const emit = defineEmits<{
  patchCommon: [id: string, key: string, value: any];
  patchKind: [id: string, key: string, value: any];
}>();

const dialog = useDialog();
const message = useMessage();
const t = useTranslate();

const w = computed(() => props.widget);

// The widget id must commit on blur, not per keystroke, or the selection
// tracking loses the widget mid edit. A plain :value input without an
// update listener reverts every keystroke, so keep a local draft.
const idDraft = ref("");
watch(
  () => w.value?.id,
  (v) => (idDraft.value = v ?? ""),
  { immediate: true },
);

// Non-null view used inside the v-if="w" branch — avoids repeating null checks
// on every inline handler. `current` is only read when `w` is non-null.
const current = computed(() => w.value as Widget);

const needsFps = computed(() => {
  const t = w.value?.kind.type;
  return t === "video" || t === "clock_digital" || t === "clock_analog";
});

const needsUpdateInterval = computed(() => {
  const t = w.value?.kind.type;
  return (
    t === "value_text" ||
    t === "radial_gauge" ||
    t === "vertical_bar" ||
    t === "horizontal_bar" ||
    t === "speedometer" ||
    t === "sparkline"
  );
});

// Keys of the kind payload (everything except `type`), in declaration order.
const kindKeys = computed<string[]>(() => {
  if (!w.value) return [];
  return Object.keys(w.value.kind).filter((k) => k !== "type");
});

function isColorKey(key: string): boolean {
  return key.endsWith("_color") || key === "color";
}
function isFontKey(key: string): boolean {
  return key === "font" || key === "numbers_font" || key === "axis_label_font";
}
function isBoolKey(key: string): boolean {
  return (
    key.startsWith("show_") ||
    key.startsWith("smooth") ||
    key.startsWith("loop") ||
    key.startsWith("fill") ||
    key.startsWith("auto_") ||
    key.startsWith("scroll") ||
    key.startsWith("range_") ||
    key.startsWith("axis_labels_on_right") ||
    key === "gradient" ||
    key === "clamp_1" ||
    key === "clamp_2"
  );
}
function isAlign(key: string) {
  return key === "align";
}
function isFit(key: string) {
  return key === "fit";
}
function isOrientation(key: string) {
  return key === "orientation";
}
function isSource(key: string) {
  return key === "source";
}
/** Arrays of `{number, color, alpha}` entries (sensor ranges, gradient stops). */
function isColorList(key: string) {
  return key === "ranges" || key === "gradient_stops";
}

function valueKind(
  key: string,
  value: unknown,
):
  | "text"
  | "number"
  | "bool"
  | "color"
  | "font"
  | "align"
  | "fit"
  | "orientation"
  | "source"
  | "colorlist" {
  if (isAlign(key)) return "align";
  if (isFit(key)) return "fit";
  if (isOrientation(key)) return "orientation";
  if (isSource(key)) return "source";
  if (isColorList(key)) return "colorlist";
  if (isColorKey(key)) return "color";
  if (isFontKey(key)) return "font";
  if (isBoolKey(key)) return "bool";
  if (typeof value === "number") return "number";
  return "text";
}

const alignOptions = computed(() => [
  { label: t("editor.properties.align.left"), value: "left" },
  { label: t("editor.properties.align.center"), value: "center" },
  { label: t("editor.properties.align.right"), value: "right" },
]);
const fitOptions = computed(() => [
  { label: t("editor.properties.fit.stretch"), value: "stretch" },
  { label: t("editor.properties.fit.contain"), value: "contain" },
  { label: t("editor.properties.fit.cover"), value: "cover" },
]);
const orientationOptions = computed(() => [
  { label: t("editor.properties.orientation.horizontal"), value: "horizontal" },
  { label: t("editor.properties.orientation.vertical"), value: "vertical" },
]);

const FIELD_LABELS: Partial<Record<string, MessageKey>> = {
  text: "editor.properties.fields.text",
  font: "editor.properties.fields.font",
  font_size: "editor.properties.fields.fontSize",
  color: "editor.properties.fields.color",
  align: "editor.properties.fields.align",
  letter_spacing: "editor.properties.fields.letterSpacing",
  source: "editor.properties.fields.source",
  format: "editor.properties.fields.format",
  unit: "editor.properties.fields.unit",
  value_min: "editor.properties.fields.valueMin",
  value_max: "editor.properties.fields.valueMax",
  start_angle: "editor.properties.fields.startAngle",
  sweep_angle: "editor.properties.fields.sweepAngle",
  inner_radius_pct: "editor.properties.fields.innerRadiusPct",
  background_color: "editor.properties.fields.backgroundColor",
  bg_corner_radius: "editor.properties.fields.bgCornerRadius",
  value_corner_radius: "editor.properties.fields.valueCornerRadius",
  gradient: "editor.properties.fields.gradient",
  corner_radius: "editor.properties.fields.cornerRadius",
  needle_color: "editor.properties.fields.needleColor",
  tick_color: "editor.properties.fields.tickColor",
  tick_count: "editor.properties.fields.tickCount",
  show_gauge: "editor.properties.fields.showGauge",
  show_needle: "editor.properties.fields.showNeedle",
  needle_width: "editor.properties.fields.needleWidth",
  needle_length_pct: "editor.properties.fields.needleLengthPct",
  needle_border_color: "editor.properties.fields.needleBorderColor",
  needle_border_width: "editor.properties.fields.needleBorderWidth",
  orientation: "editor.properties.fields.orientation",
  show_labels: "editor.properties.fields.showLabels",
  path: "editor.properties.fields.path",
  opacity: "editor.properties.fields.opacity",
  fit: "editor.properties.fields.fit",
  loop_playback: "editor.properties.fields.loopPlayback",
  auto_range: "editor.properties.fields.autoRange",
  history_length: "editor.properties.fields.historyLength",
  line_width: "editor.properties.fields.lineWidth",
  line_color: "editor.properties.fields.lineColor",
  fill_color: "editor.properties.fields.fillColor",
  border_color: "editor.properties.fields.borderColor",
  border_width: "editor.properties.fields.borderWidth",
  padding: "editor.properties.fields.padding",
  show_points: "editor.properties.fields.showPoints",
  point_radius: "editor.properties.fields.pointRadius",
  show_baseline: "editor.properties.fields.showBaseline",
  baseline_value: "editor.properties.fields.baselineValue",
  baseline_color: "editor.properties.fields.baselineColor",
  baseline_width: "editor.properties.fields.baselineWidth",
  smooth: "editor.properties.fields.smooth",
  scroll_rtl: "editor.properties.fields.scrollRtl",
  fill_from_ranges: "editor.properties.fields.fillFromRanges",
  range_blend: "editor.properties.fields.rangeBlend",
  show_gridlines: "editor.properties.fields.showGridlines",
  gridlines_horizontal: "editor.properties.fields.gridlinesHorizontal",
  gridlines_vertical: "editor.properties.fields.gridlinesVertical",
  gridline_color: "editor.properties.fields.gridlineColor",
  gridline_width: "editor.properties.fields.gridlineWidth",
  show_axis_labels: "editor.properties.fields.showAxisLabels",
  axis_label_count: "editor.properties.fields.axisLabelCount",
  axis_labels_on_right: "editor.properties.fields.axisLabelsOnRight",
  axis_label_format: "editor.properties.fields.axisLabelFormat",
  axis_label_font: "editor.properties.fields.axisLabelFont",
  axis_label_size: "editor.properties.fields.axisLabelSize",
  axis_label_color: "editor.properties.fields.axisLabelColor",
  axis_label_padding: "editor.properties.fields.axisLabelPadding",
  face_color: "editor.properties.fields.faceColor",
  minor_tick_color: "editor.properties.fields.minorTickColor",
  hour_hand_color: "editor.properties.fields.hourHandColor",
  minute_hand_color: "editor.properties.fields.minuteHandColor",
  second_hand_color: "editor.properties.fields.secondHandColor",
  hub_color: "editor.properties.fields.hubColor",
  numbers_color: "editor.properties.fields.numbersColor",
  numbers_font: "editor.properties.fields.numbersFont",
  numbers_font_size: "editor.properties.fields.numbersFontSize",
  show_seconds: "editor.properties.fields.showSeconds",
  show_hour_ticks: "editor.properties.fields.showHourTicks",
  show_minor_ticks: "editor.properties.fields.showMinorTicks",
  show_numbers: "editor.properties.fields.showNumbers",
  hour_hand_width: "editor.properties.fields.hourHandWidth",
  minute_hand_width: "editor.properties.fields.minuteHandWidth",
  second_hand_width: "editor.properties.fields.secondHandWidth",
  hour_hand_length_pct: "editor.properties.fields.hourHandLengthPct",
  minute_hand_length_pct: "editor.properties.fields.minuteHandLengthPct",
  second_hand_length_pct: "editor.properties.fields.secondHandLengthPct",
  hour_tick_length_pct: "editor.properties.fields.hourTickLengthPct",
  minor_tick_length_pct: "editor.properties.fields.minorTickLengthPct",
  hour_tick_width: "editor.properties.fields.hourTickWidth",
  minor_tick_width: "editor.properties.fields.minorTickWidth",
  hub_radius: "editor.properties.fields.hubRadius",
};

function fieldLabel(key: string): string {
  const label = FIELD_LABELS[key];
  if (label) return t(label);
  return key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

// ── Source dropdown ────────────────────────────────────────────────────────
// The stored source may not exactly match any enumerated sensor (e.g. a custom
// command or a hwmon path from another machine), so NSelect would show the raw
// JSON. We inject the current source as an extra option with a friendly label
// so the dropdown always displays something readable.
function sourceLabel(src: SensorSourceConfig | null | undefined): string {
  if (!src) return "—";
  switch (src.type) {
    case "hwmon":
      return `${src.name}: ${src.label}`;
    case "command":
      return t("editor.properties.sources.command", { cmd: (src.cmd || "").slice(0, 24) });
    case "constant":
      return t("editor.properties.sources.constant", { value: src.value });
    case "nvidia_gpu":
      return t("editor.properties.sources.nvidiaGpu", { index: src.gpu_index ?? 0, metric: src.metric ?? "temp" });
    case "amd_gpu_usage":
      return t("editor.properties.sources.amdGpuUsage", { index: src.card_index ?? 0 });
    case "wireless_coolant":
      return t("editor.properties.sources.coolant", { device: src.device_id });
    case "cpu_usage":
      return t("editor.properties.sources.cpuUsage");
    case "mem_usage":
      return t("editor.properties.sources.memUsage");
    case "mem_used":
      return t("editor.properties.sources.memUsed");
    case "mem_free":
      return t("editor.properties.sources.memFree");
    case "network_rx":
      return t("editor.properties.sources.networkRx", { iface: src.iface });
    case "network_tx":
      return t("editor.properties.sources.networkTx", { iface: src.iface });
    case "disk_read":
      return t("editor.properties.sources.diskRead", { device: src.device });
    case "disk_write":
      return t("editor.properties.sources.diskWrite", { device: src.device });
    default:
      return (src as any).type ?? t("editor.properties.sources.fallback");
  }
}

const UNBOUND = "unbound";

function sourceOptionsFor(key: string) {
  const src = (current.value.kind as any)[key] as SensorSourceConfig | undefined;
  const opts = [...props.sensorOptions];
  const category = current.value.sensor_category;
  if (category) {
    const label = t("editor.properties.sources.unbound", { category: t(SENSOR_CATEGORY_LABELS[category]) });
    return [{ label, value: UNBOUND }, ...opts];
  }
  if (src?.type === "command") return opts;
  const json = src ? JSON.stringify(src) : "";
  if (json && !opts.some((o) => o.value === json)) {
    opts.unshift({ label: sourceLabel(src), value: json });
  }
  return opts;
}

function sourceSelectValue(key: string): string {
  if (current.value.sensor_category) return UNBOUND;
  const src = (current.value.kind as any)[key] as SensorSourceConfig | undefined;
  if (src?.type === "command") return "command";
  return src ? JSON.stringify(src) : "";
}

function onSourceSelect(key: string, v: string) {
  if (v === UNBOUND) return;
  if (current.value.sensor_category) emit("patchCommon", current.value.id, "sensor_category", null);
  if (v === "command") {
    const src = (current.value.kind as any)[key] as SensorSourceConfig | undefined;
    const cmd = src?.type === "command" ? src.cmd : "";
    emit("patchKind", current.value.id, key, { type: "command", cmd });
    return;
  }
  try {
    emit("patchKind", current.value.id, key, JSON.parse(v));
  } catch {}
}

function isCommandSource(key: string): boolean {
  return ((current.value.kind as any)[key] as SensorSourceConfig | undefined)?.type === "command";
}

// ── Color list editors (ranges / gradient stops) ───────────────────────────
function listNumKey(key: string): "max" | "position" {
  return key === "ranges" ? "max" : "position";
}
function listNumLabel(key: string): string {
  return key === "ranges" ? t("editor.properties.rangeMax") : t("editor.properties.stopPosition");
}
function listHeadLabel(key: string): string {
  if (key === "gradient_stops") return t("editor.properties.gradientStops");
  if (key === "ranges") return t("editor.properties.gaugeRanges");
  return fieldLabel(key);
}
function listNumMin(key: string): number {
  return key === "ranges" ? -1 : 0;
}
function listNumValue(key: string, item: SensorRange | GradientStop): number | null {
  const v = (item as any)[listNumKey(key)];
  if (key === "ranges" && (v === null || v === undefined)) return -1;
  return v ?? 0;
}
function commitListNum(key: string, v: number | null): number | null {
  if (key === "ranges" && v !== null && v < 0) return null;
  return v;
}

// ── Number field formatting ────────────────────────────────────────────────
// Some numeric fields are stored as 0..1 ratios (inner_radius_pct, *_length_pct)
// but should be edited/displayed as 0–100%; angles are 0–360.
function isPctKey(key: string): boolean {
  return key.endsWith("_pct");
}
function isAngleKey(key: string): boolean {
  return key === "start_angle" || key === "sweep_angle";
}
function numDisplay(key: string, value: number): number {
  return isPctKey(key) ? Math.round(value * 100) : value;
}
function numMax(key: string): number | undefined {
  if (isPctKey(key)) return 100;
  if (isAngleKey(key)) return 360;
  return undefined;
}
function numLabel(key: string): string {
  return isPctKey(key) ? t("editor.properties.percentLabel", { label: fieldLabel(key) }) : fieldLabel(key);
}
function numCommit(key: string, v: number | null): number {
  const n = v ?? 0;
  return isPctKey(key) ? n / 100 : n;
}
function listItemColor(item: SensorRange | GradientStop): RGBA {
  const c = (item as any).color as RGB;
  return [c[0], c[1], c[2], (item as any).alpha ?? 255];
}
function patchListItem(key: string, index: number, partial: Record<string, unknown>) {
  const arr = [...((current.value.kind as any)[key] as unknown[])];
  arr[index] = { ...(arr[index] as object), ...partial };
  emit("patchKind", current.value.id, key, arr);
}
function addListItem(key: string) {
  const arr = [...((current.value.kind as any)[key] as unknown[])];
  if (key === "ranges") {
    arr.push({ max: 100, color: [255, 255, 255], alpha: 255 });
  } else {
    arr.push({ position: 50, color: [255, 255, 255], alpha: 255 });
  }
  emit("patchKind", current.value.id, key, arr);
}
function removeListItem(key: string, index: number) {
  dialog.error({
    title: t("editor.properties.removeItemTitle"),
    content: t("editor.properties.removeItemContent", { index: index + 1 }),
    positiveText: t("common.remove"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => {
      const arr = ((current.value.kind as any)[key] as unknown[]).filter((_, i) => i !== index);
      emit("patchKind", current.value.id, key, arr);
    },
  });
}

async function browsePath(key: string) {
  const widget = w.value;
  if (!widget || (widget.kind.type !== "image" && widget.kind.type !== "video")) return;
  const type = widget.kind.type;
  try {
    const selected = await pickMediaFile(type === "video" ? "animation" : "image");
    if (selected && w.value === widget && widget.kind.type === type) emit("patchKind", widget.id, key, selected);
  } catch (error) {
    message.error(String(error));
  }
}
</script>

<template>
  <div class="props">
    <template v-if="w">
      <div class="group-title">{{ t("editor.properties.common") }}</div>
      <div class="field"><label class="muted">ID</label>
        <n-input v-model:value="idDraft" size="small" @blur="emit('patchCommon', current.id, 'id', idDraft)" />
      </div>
      <div class="grid4">
        <div class="field"><label class="muted">X</label>
          <n-input-number :value="current.x" size="small" @update:value="(v) => emit('patchCommon', current.id, 'x', v ?? 0)" />
        </div>
        <div class="field"><label class="muted">Y</label>
          <n-input-number :value="current.y" size="small" @update:value="(v) => emit('patchCommon', current.id, 'y', v ?? 0)" />
        </div>
        <div class="field"><label class="muted">{{ t("editor.properties.width") }}</label>
          <n-input-number :value="current.width" size="small" :min="1" @update:value="(v) => emit('patchCommon', current.id, 'width', v ?? 1)" />
        </div>
        <div class="field"><label class="muted">{{ t("editor.properties.height") }}</label>
          <n-input-number :value="current.height" size="small" :min="1" @update:value="(v) => emit('patchCommon', current.id, 'height', v ?? 1)" />
        </div>
      </div>
      <div class="grid4">
        <div class="field"><label class="muted">{{ t("editor.properties.rotation") }}</label>
          <n-input-number :value="current.rotation ?? 0" size="small" @update:value="(v) => emit('patchCommon', current.id, 'rotation', v ?? 0)" />
        </div>
        <div class="field check"><n-checkbox :checked="current.visible !== false" @update:checked="(v) => emit('patchCommon', current.id, 'visible', v)">{{ t("editor.properties.visible") }}</n-checkbox></div>
      </div>
      <div v-if="needsFps" class="field"><label class="muted">FPS</label>
        <n-input-number :value="current.fps ?? 30" size="small" :min="1" :max="120" @update:value="(v) => emit('patchCommon', current.id, 'fps', v ?? 30)" />
      </div>
      <div v-if="needsUpdateInterval" class="field"><label class="muted">{{ t("editor.properties.updateInterval") }}</label>
        <n-input-number :value="current.update_interval_ms ?? 1000" size="small" :min="100" :max="10000" :step="100" @update:value="(v) => emit('patchCommon', current.id, 'update_interval_ms', v ?? 1000)" />
      </div>

      <div class="group-title">{{ current.kind.type }}</div>
      <template v-for="key in kindKeys" :key="key">
        <!-- Source (sensor) -->
        <div v-if="valueKind(key, (current.kind as any)[key]) === 'source'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <SensorSelect
            size="small"
            :value="sourceSelectValue(key)"
            :options="sourceOptionsFor(key)"
            filterable
            @update:value="(v) => onSourceSelect(key, v)"
          />
          <n-input
            v-if="isCommandSource(key)"
            size="small"
            :placeholder="t('editor.properties.commandPlaceholder')"
            :value="(current.kind as any)[key].cmd"
            @update:value="(v) => emit('patchKind', current.id, key, { type: 'command', cmd: v })"
          />
        </div>
        <!-- Color list (sensor ranges / gradient stops) -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'colorlist'" class="field">
          <div class="list-head">
            <label class="muted">{{ listHeadLabel(key) }}</label>
            <n-button size="tiny" quaternary @click="addListItem(key)">
              <template #icon><Plus :size="12" /></template>{{ t("common.add") }}
            </n-button>
          </div>
          <div
            v-for="(item, i) in (current.kind as any)[key]"
            :key="i"
            class="list-row"
          >
            <n-input-number
              :value="listNumValue(key, item)"
              size="small"
              :placeholder="listNumLabel(key)"
              :min="listNumMin(key)"
              @update:value="(v) => patchListItem(key, i, { [listNumKey(key)]: commitListNum(key, v) })"
            />
            <ColorInline
              :model-value="listItemColor(item)"
              @update:model-value="(v) => patchListItem(key, i, { color: [v[0], v[1], v[2]], alpha: v[3] ?? 255 })"
            />
            <n-button size="tiny" quaternary type="error" @click="removeListItem(key, i)">
              <template #icon><Trash2 :size="12" /></template>
            </n-button>
          </div>
          <div v-if="key === 'ranges'" class="hint muted">{{ t("editor.properties.rangesHint") }}</div>
        </div>
        <!-- Color -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'color'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <ColorInline :model-value="(current.kind as any)[key]" @update:model-value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <!-- Bool -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'bool'" class="field check">
          <n-checkbox :checked="(current.kind as any)[key]" @update:checked="(v) => emit('patchKind', current.id, key, v)">{{ fieldLabel(key) }}</n-checkbox>
        </div>
        <!-- Align / Fit / Orientation -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'align'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <n-select size="small" :value="(current.kind as any)[key]" :options="alignOptions" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'fit'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <n-select size="small" :value="(current.kind as any)[key]" :options="fitOptions" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'orientation'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <n-select size="small" :value="(current.kind as any)[key]" :options="orientationOptions" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <!-- Number -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'number'" class="field">
          <label class="muted">{{ numLabel(key) }}</label>
          <n-input-number
            :value="numDisplay(key, (current.kind as any)[key])"
            :max="numMax(key)"
            size="small"
            @update:value="(v) => emit('patchKind', current.id, key, numCommit(key, v))"
          />
        </div>
        <!-- Font / path-like text -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'font'" class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <n-select
            size="small"
            :value="(current.kind as any)[key]?.path ?? ''"
            :options="fontOptions"
            filterable
            @update:value="(v) => emit('patchKind', current.id, key, { path: v || null })"
          />
        </div>
        <!-- Text (incl. path fields) -->
        <div v-else class="field">
          <label class="muted">{{ fieldLabel(key) }}</label>
          <div class="path-row" v-if="key === 'path'">
            <n-input :value="(current.kind as any)[key]" size="small" @update:value="(v) => emit('patchKind', current.id, key, v)" />
            <n-button size="small" @click="browsePath(key)"><template #icon><FolderOpen :size="14" /></template></n-button>
          </div>
          <n-input v-else :value="(current.kind as any)[key]" size="small" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
      </template>
    </template>
    <div v-else class="empty muted">{{ t("editor.properties.empty") }}</div>
  </div>
</template>

<script lang="ts">
// Inline color editor for the properties panel (kept simple — hex only).
import { defineComponent, h } from "vue";
import { NColorPicker } from "naive-ui";
const ColorInline = defineComponent({
  name: "ColorInline",
  props: { modelValue: { type: Array as any, default: () => [0, 0, 0, 255] } },
  emits: ["update:modelValue"],
  setup(p, { emit }) {
    return () => {
      const c = p.modelValue as RGBA;
      const hex =
        "#" +
        [c[0], c[1], c[2], c[3] ?? 255]
          .map((n) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0"))
          .join("");
      return h(NColorPicker, {
        value: hex,
        showAlpha: true,
        modes: ["hex"],
        size: "small",
        onUpdateValue: (v: string) => {
          const r = parseInt(v.slice(1, 3), 16);
          const g = parseInt(v.slice(3, 5), 16);
          const b = parseInt(v.slice(5, 7), 16);
          const a = v.length >= 9 ? parseInt(v.slice(7, 9), 16) : 255;
          emit("update:modelValue", [r, g, b, a]);
        },
      });
    };
  },
});
export default { components: { ColorInline } };
</script>

<style scoped>
.props {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  overflow-y: auto;
  height: 100%;
}
.group-title {
  font-size: var(--font-size-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--md-on-surface-variant);
  margin-top: var(--space-3);
  padding-bottom: var(--space-2);
  border-bottom: 1px solid var(--md-outline-variant);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.field > label {
  font-size: var(--font-size-xs);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.field.check {
  flex-direction: row;
  align-items: center;
  /* Center vertically against the adjacent labeled input in the grid row. */
  align-self: center;
}
.grid4 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-2);
}
.path-row {
  display: flex;
  gap: var(--space-1);
}
.list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.list-row {
  display: flex;
  align-items: flex-end;
  gap: var(--space-1);
}
.empty {
  padding: var(--space-4);
  font-size: var(--font-size-sm);
}
.hint {
  font-size: var(--font-size-xs);
  line-height: 1.4;
}
</style>
