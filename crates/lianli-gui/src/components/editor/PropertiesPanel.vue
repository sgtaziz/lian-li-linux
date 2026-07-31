<script setup lang="ts">
import { computed } from "vue";
import { useDialog } from "naive-ui";
import { FolderOpen, Plus, Trash2 } from "lucide-vue-next";
import type { Widget, RGBA, RGB, SensorSourceConfig, SensorRange, GradientStop } from "@/types";
import { open } from "@tauri-apps/plugin-dialog";

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

const w = computed(() => props.widget);

// Non-null view used inside the v-if="w" branch — avoids repeating null checks
// on every inline handler. `current` is only read when `w` is non-null.
const current = computed(() => w.value as Widget);

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

const alignOptions = [
  { label: "Left", value: "left" },
  { label: "Center", value: "center" },
  { label: "Right", value: "right" },
];
const fitOptions = [
  { label: "Stretch", value: "stretch" },
  { label: "Contain", value: "contain" },
  { label: "Cover", value: "cover" },
];
const orientationOptions = [
  { label: "Horizontal", value: "horizontal" },
  { label: "Vertical", value: "vertical" },
];

function pretty(key: string): string {
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
      return `command (${(src.cmd || "").slice(0, 24)})`;
    case "constant":
      return `constant ${src.value}`;
    case "nvidia_gpu":
      return `NVIDIA GPU ${src.gpu_index ?? 0} (${src.metric ?? "temp"})`;
    case "amd_gpu_usage":
      return `AMD GPU ${src.card_index ?? 0}`;
    case "wireless_coolant":
      return `coolant ${src.device_id}`;
    case "cpu_usage":
      return "CPU Usage";
    case "mem_usage":
      return "Memory Usage";
    case "mem_used":
      return "Memory Used";
    case "mem_free":
      return "Memory Free";
    case "network_rx":
    case "network_tx":
      return `net ${src.iface}`;
    case "disk_read":
    case "disk_write":
      return `disk ${src.device}`;
    default:
      return (src as any).type ?? "source";
  }
}

function sourceOptionsFor(key: string) {
  const src = (current.value.kind as any)[key] as SensorSourceConfig | undefined;
  const json = src ? JSON.stringify(src) : "";
  const opts = [...props.sensorOptions];
  if (json && !opts.some((o) => o.value === json)) {
    opts.unshift({ label: sourceLabel(src), value: json });
  }
  return opts;
}

// ── Color list editors (ranges / gradient stops) ───────────────────────────
function listNumKey(key: string): "max" | "position" {
  return key === "ranges" ? "max" : "position";
}
function listNumLabel(key: string): string {
  return key === "ranges" ? "max %" : "pos %";
}
function listHeadLabel(key: string): string {
  if (key === "gradient_stops") return "Gradient Stops (% of Value Max)";
  if (key === "ranges") return "Gauge Ranges";
  return pretty(key);
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
  return isPctKey(key) ? `${pretty(key)} (%)` : pretty(key);
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
    title: "Remove item?",
    content: `Item ${index + 1} will be removed.`,
    positiveText: "Remove",
    negativeText: "Cancel",
    onPositiveClick: () => {
      const arr = ((current.value.kind as any)[key] as unknown[]).filter((_, i) => i !== index);
      emit("patchKind", current.value.id, key, arr);
    },
  });
}

async function browsePath(key: string) {
  const sel = await open({ filters: [{ name: "Files", extensions: ["png", "jpg", "jpeg", "bmp", "gif", "mp4", "webm", "ttf", "otf"] }] });
  if (typeof sel === "string") emit("patchKind", w.value!.id, key, sel);
}
</script>

<template>
  <div class="props">
    <template v-if="w">
      <div class="group-title">Common</div>
      <div class="field"><label class="muted">ID</label>
        <n-input :value="current.id" size="small" @blur="emit('patchCommon', current.id, 'id', ($event.target as HTMLInputElement).value)" />
      </div>
      <div class="grid4">
        <div class="field"><label class="muted">X</label>
          <n-input-number :value="current.x" size="small" @update:value="(v) => emit('patchCommon', current.id, 'x', v ?? 0)" />
        </div>
        <div class="field"><label class="muted">Y</label>
          <n-input-number :value="current.y" size="small" @update:value="(v) => emit('patchCommon', current.id, 'y', v ?? 0)" />
        </div>
        <div class="field"><label class="muted">W</label>
          <n-input-number :value="current.width" size="small" :min="1" @update:value="(v) => emit('patchCommon', current.id, 'width', v ?? 1)" />
        </div>
        <div class="field"><label class="muted">H</label>
          <n-input-number :value="current.height" size="small" :min="1" @update:value="(v) => emit('patchCommon', current.id, 'height', v ?? 1)" />
        </div>
      </div>
      <div class="grid4">
        <div class="field"><label class="muted">Rotation</label>
          <n-input-number :value="current.rotation ?? 0" size="small" @update:value="(v) => emit('patchCommon', current.id, 'rotation', v ?? 0)" />
        </div>
        <div class="field check"><n-checkbox :checked="current.visible !== false" @update:checked="(v) => emit('patchCommon', current.id, 'visible', v)">Visible</n-checkbox></div>
      </div>

      <div class="group-title">{{ current.kind.type }}</div>
      <template v-for="key in kindKeys" :key="key">
        <!-- Source (sensor) -->
        <div v-if="valueKind(key, (current.kind as any)[key]) === 'source'" class="field">
          <label class="muted">{{ pretty(key) }}</label>
          <n-select
            size="small"
            :value="JSON.stringify((current.kind as any)[key])"
            :options="sourceOptionsFor(key)"
            filterable
            @update:value="(v) => { try { emit('patchKind', current.id, key, JSON.parse(v)); } catch {} }"
          />
        </div>
        <!-- Color list (sensor ranges / gradient stops) -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'colorlist'" class="field">
          <div class="list-head">
            <label class="muted">{{ listHeadLabel(key) }}</label>
            <n-button size="tiny" quaternary @click="addListItem(key)">
              <template #icon><Plus :size="12" /></template>Add
            </n-button>
          </div>
          <div
            v-for="(item, i) in (current.kind as any)[key]"
            :key="i"
            class="list-row"
          >
            <n-input-number
              :value="item[listNumKey(key)]"
              size="small"
              :placeholder="listNumLabel(key)"
              :min="0"
              @update:value="(v) => patchListItem(key, i, { [listNumKey(key)]: v })"
            />
            <ColorInline
              :model-value="listItemColor(item)"
              @update:model-value="(v) => patchListItem(key, i, { color: [v[0], v[1], v[2]], alpha: v[3] ?? 255 })"
            />
            <n-button size="tiny" quaternary type="error" @click="removeListItem(key, i)">
              <template #icon><Trash2 :size="12" /></template>
            </n-button>
          </div>
        </div>
        <!-- Color -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'color'" class="field">
          <label class="muted">{{ pretty(key) }}</label>
          <ColorInline :model-value="(current.kind as any)[key]" @update:model-value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <!-- Bool -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'bool'" class="field check">
          <n-checkbox :checked="(current.kind as any)[key]" @update:checked="(v) => emit('patchKind', current.id, key, v)">{{ pretty(key) }}</n-checkbox>
        </div>
        <!-- Align / Fit / Orientation -->
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'align'" class="field">
          <label class="muted">{{ pretty(key) }}</label>
          <n-select size="small" :value="(current.kind as any)[key]" :options="alignOptions" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'fit'" class="field">
          <label class="muted">{{ pretty(key) }}</label>
          <n-select size="small" :value="(current.kind as any)[key]" :options="fitOptions" @update:value="(v) => emit('patchKind', current.id, key, v)" />
        </div>
        <div v-else-if="valueKind(key, (current.kind as any)[key]) === 'orientation'" class="field">
          <label class="muted">{{ pretty(key) }}</label>
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
          <label class="muted">{{ pretty(key) }}</label>
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
          <label class="muted">{{ pretty(key) }}</label>
          <div class="path-row" v-if="key === 'path'">
            <n-input :value="(current.kind as any)[key]" size="small" @blur="emit('patchKind', current.id, key, ($event.target as HTMLInputElement).value)" />
            <n-button size="small" @click="browsePath(key)"><template #icon><FolderOpen :size="14" /></template></n-button>
          </div>
          <n-input v-else :value="(current.kind as any)[key]" size="small" @blur="emit('patchKind', current.id, key, ($event.target as HTMLInputElement).value)" />
        </div>
      </template>
    </template>
    <div v-else class="empty muted">Select a widget to edit its properties.</div>
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
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-muted);
  margin-top: var(--space-2);
  padding-bottom: var(--space-1);
  border-bottom: 1px solid var(--border);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
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
</style>
