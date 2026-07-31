<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { FolderOpen, Trash2 } from "lucide-vue-next";
import type { DeviceInfo, LcdConfig, MediaType, SensorDescriptor } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdStore } from "@/stores/lcd";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import { open } from "@tauri-apps/plugin-dialog";
import SensorGaugeEditor from "@/components/lcd/SensorGaugeEditor.vue";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import OrientationPicker from "@/components/common/OrientationPicker.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { enumerateSensorsAsOptions, optionForConfig, decodeOption } from "@/stores/sensorOptions";
import { screenSupportsH264 } from "@/constants/screen";

const props = defineProps<{
  entry: LcdConfig;
  index: number;
}>();

const config = useConfigStore();
const devices = useDevicesStore();
const lcd = useLcdStore();
const ipc = useIpc();
const dialog = useDialog();

const lcdDevices = computed(() => devices.lcdDevices);

// Stable ordering + disambiguation: sort by serial, and when more than one
// device shares the same name append "#N" so identical fans are distinguishable.
const deviceOptions = computed(() => {
  const sorted = [...lcdDevices.value].sort((a, b) =>
    (a.serial ?? a.device_id).localeCompare(b.serial ?? b.device_id),
  );
  const nameCounts = new Map<string, number>();
  for (const d of sorted) nameCounts.set(d.name, (nameCounts.get(d.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  return sorted.map((d) => {
    const dup = (nameCounts.get(d.name) ?? 1) > 1;
    const n = (seen.get(d.name) ?? 0) + 1;
    seen.set(d.name, n);
    return { label: dup ? `${d.name} #${n}` : d.name, value: d.device_id };
  });
});

// Resolve the LCD entry to the concrete device it targets.
function deviceForEntry(): DeviceInfo | undefined {
  if (props.entry.serial) {
    return lcdDevices.value.find((d) => d.serial === props.entry.serial);
  }
  const idx = props.entry.index ?? 0;
  return lcdDevices.value[idx];
}
const selectedDeviceId = computed(
  () => deviceForEntry()?.device_id ?? lcdDevices.value[0]?.device_id ?? "",
);
const selectedDevice = computed<DeviceInfo | undefined>(() => deviceForEntry());

function onSelectDevice(id: string) {
  const d = lcdDevices.value.find((x) => x.device_id === id);
  if (!d) return;
  props.entry.serial = d.serial;
  // For serial-less devices, record the position so the match is stable.
  props.entry.index = d.serial ? undefined : lcdDevices.value.indexOf(d);
  config.markDirty();
}

// Ensure the entry targets a real device: prefer serial, fall back to first.
watch(
  () => lcdDevices.value,
  (devs) => {
    if (devs.length === 0) return;
    if (!deviceForEntry()) {
      props.entry.serial = devs[0].serial;
      props.entry.index = devs[0].serial ? undefined : 0;
      config.markDirty();
    }
  },
  { immediate: true },
);

const mediaTypeOptions = [
  { label: "Image", value: "image" },
  { label: "Video", value: "video" },
  { label: "GIF", value: "gif" },
  { label: "Solid Color", value: "color" },
  { label: "Sensor Gauge", value: "sensor" },
  { label: "Custom Template", value: "custom" },
] as const;

function onMediaType(v: MediaType) {
  props.entry.type = v;
  config.markDirty();
}

// Focus preservation: text inputs bind to local refs, sync to entry on blur.
const localPath = ref(props.entry.path ?? "");
watch(() => props.entry.path, (v) => { localPath.value = v ?? ""; });
function commitPath() {
  props.entry.path = localPath.value || null;
  config.markDirty();
}

async function browsePath() {
  const selected = await open({
    filters: [{ name: "Media", extensions: ["png", "jpg", "jpeg", "bmp", "gif", "mp4", "webm", "mkv"] }],
  });
  if (typeof selected === "string") {
    localPath.value = selected;
    commitPath();
  }
}

// Solid color
function onColorR(v: number | null) { setColor(0, v); }
function onColorG(v: number | null) { setColor(1, v); }
function onColorB(v: number | null) { setColor(2, v); }
function setColor(i: number, v: number | null) {
  const cur = props.entry.rgb ?? [0, 0, 0];
  const next = [...cur] as [number, number, number];
  next[i] = v ?? 0;
  props.entry.rgb = next;
  config.markDirty();
}

// Sensor
const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, true));
function ensureSensor(): SensorDescriptor {
  if (!props.entry.sensor) {
    props.entry.sensor = {
      label: "CPU Temp",
      unit: "°C",
      source: { type: "cpu_usage" },
      text_color: [255, 255, 255],
      background_color: [0, 0, 0],
      gauge_background_color: [60, 60, 60],
      gauge_ranges: [
        { max: 50, color: [0, 200, 0], alpha: 255 },
        { max: 80, color: [220, 140, 0], alpha: 255 },
        { max: null, color: [220, 0, 0], alpha: 255 },
      ],
      gauge_start_angle: 90,
      gauge_sweep_angle: 330,
      gauge_outer_radius: 180,
      gauge_thickness: 40,
      bar_corner_radius: 0,
      value_font_size: 72,
      unit_font_size: 32,
      label_font_size: 28,
      font_path: null,
      decimal_places: 0,
      value_offset: 0,
      unit_offset: 60,
      label_offset: -60,
    };
  }
  return props.entry.sensor;
}

function sensorSourceValue(): string {
  return props.entry.sensor ? optionForConfig(config.sensors, props.entry.sensor.source) : "";
}
function onSensorSource(v: string) {
  const s = ensureSensor();
  s.source = decodeOption(v) ?? { type: "command", cmd: "" };
  config.markDirty();
}
const localCommand = ref("");
watch(() => props.entry.sensor?.source, () => {
  const src = props.entry.sensor?.source;
  localCommand.value = src && src.type === "command" ? src.cmd : "";
}, { immediate: true });
function commitCommand() {
  const s = ensureSensor();
  if (localCommand.value) s.source = { type: "command", cmd: localCommand.value };
  config.markDirty();
}

// Custom template sub-section
const templateOptions = computed(() =>
  config.templates.map((t) => ({ label: t.name, value: t.id })),
);
// H264 streaming is gated by the device family's screen capabilities
// (only the AIO 480×480 LCDs support it) — mirrors screen_info_for(family).h264.
const deviceSupportsH264 = computed(() =>
  screenSupportsH264(selectedDevice.value?.family ?? ("Ene6k77" as any)),
);
const supportsCCommand = computed(() => selectedDevice.value?.supports_c_command ?? false);

function onTemplateId(v: string) {
  props.entry.template_id = v || null;
  config.markDirty();
}

// Live preview of the selected template, rendered by the daemon
// (RenderTemplatePreview), debounced 200ms — matches the Slint LCD card preview.
const previewJpeg = ref("");
const previewLoading = ref(false);
const renderPreview = useDebounce(async () => {
  const id = props.entry.template_id;
  const tpl = id ? config.templates.find((t) => t.id === id) : undefined;
  if (!tpl) {
    previewJpeg.value = "";
    return;
  }
  previewLoading.value = true;
  try {
    const res = await ipc.request<{ jpeg_base64: string }>("RenderTemplatePreview", {
      template: tpl,
      width: tpl.base_width,
      height: tpl.base_height,
    });
    previewJpeg.value = res.jpeg_base64 ?? "";
  } catch {
    previewJpeg.value = "";
  } finally {
    previewLoading.value = false;
  }
}, 200);

watch(
  () => [props.entry.template_id, props.entry.type] as const,
  () => {
    if (props.entry.type === "custom" && props.entry.template_id) renderPreview();
    else previewJpeg.value = "";
  },
  { immediate: true },
);

function nextUniqueName(base: string, taken: string[]): string {
  const stem = stripCopySuffix(base);
  const names = new Set(taken);
  if (!names.has(stem) && stem !== base) return stem;
  const first = `${stem} (Copy)`;
  if (!names.has(first)) return first;
  for (let i = 2; i < 1000; i++) {
    const c = `${stem} (Copy ${i})`;
    if (!names.has(c)) return c;
  }
  return `${stem} (Copy ${Date.now().toString(16)})`;
}
function stripCopySuffix(name: string): string {
  const idx = name.lastIndexOf(" (Copy");
  if (idx >= 0) {
    const tail = name.slice(idx + 6);
    if (tail === ")" || (tail.startsWith(" ") && tail.endsWith(")"))) {
      return name.slice(0, idx);
    }
  }
  return name;
}

/** Duplicate the entry's currently-selected template into a new user template. */
async function duplicateTemplate() {
  const id = props.entry.template_id;
  const src = id ? config.templates.find((t) => t.id === id) : undefined;
  if (!src) return;
  const copy = JSON.parse(JSON.stringify(src));
  copy.id = "user-" + Date.now().toString(16);
  copy.name = nextUniqueName(src.name, config.templates.map((t) => t.name));
  const newId = copy.id;
  config.templates.push(copy);
  props.entry.template_id = newId;
  await lcd.setTemplates(config.templates);
  await config.load();
  config.markDirty();
  renderPreview();
}

/** Delete the entry's currently-selected template (clears all references). */
async function deleteTemplate() {
  const id = props.entry.template_id;
  if (!id) return;
  dialog.error({
    title: "Delete template?",
    content: "This template will be permanently deleted and removed from all LCD entries.",
    positiveText: "Delete",
    negativeText: "Cancel",
    onPositiveClick: async () => {
      config.templates = config.templates.filter((t) => t.id !== id);
      for (const e of config.config.lcds) {
        if (e.template_id === id) e.template_id = null;
      }
      await lcd.setTemplates(config.templates);
      await config.load();
      config.markDirty();
      renderPreview();
    },
  });
}

/** "Edit" opens the editor on this entry's currently-selected template. */
async function openEditor(templateId?: string) {
  await ipc.openEditorWindow(templateId);
}
async function openBrowser() {
  await ipc.openBrowserWindow();
}

function removeEntry() {
  dialog.error({
    title: "Remove LCD entry?",
    content: `LCD ${props.index + 1} will be removed from the configuration.`,
    positiveText: "Remove",
    negativeText: "Cancel",
    onPositiveClick: () => {
      config.config.lcds.splice(props.index, 1);
      config.markDirty();
    },
  });
}

// FPS / update interval / orientation
function onFps(v: number | null) {
  props.entry.fps = v;
  config.markDirty();
}
function onUpdateInterval(v: number | null) {
  props.entry.update_interval_ms = v ?? undefined;
  config.markDirty();
}
function onOrientation(v: number) {
  props.entry.orientation = v;
  config.markDirty();
}

const brightness = computed({
  get: () => props.entry.brightness ?? 100,
  set: (v: number) => {
    props.entry.brightness = v;
    config.markDirty();
    if (selectedDeviceId.value) {
      void lcd.setBrightness(selectedDeviceId.value, v);
    }
  },
});
</script>

<script lang="ts">
// (LcdConfig helpers live in the setup script above.)
</script>

<template>
  <div class="card lcd-config">
    <div class="head">
      <span class="title">LCD {{ index + 1 }}</span>
      <n-button size="small" quaternary type="error" @click="removeEntry">
        <template #icon><Trash2 :size="14" /></template>
      </n-button>
    </div>

    <div class="grid">
      <div class="field">
        <label class="muted">Device</label>
        <n-select
          :value="selectedDeviceId"
          :options="deviceOptions"
          size="small"
          filterable
          @update:value="onSelectDevice"
        />
      </div>
      <div class="field">
        <label class="muted">Media type</label>
        <n-select :value="entry.type" :options="mediaTypeOptions" size="small" @update:value="onMediaType" />
      </div>
    </div>

    <!-- Image / Video / GIF -->
    <div v-if="['image', 'video', 'gif'].includes(entry.type)" class="field">
      <label class="muted">Path</label>
      <div class="path-row">
        <n-input v-model:value="localPath" @blur="commitPath" size="small" placeholder="/path/to/media" />
        <n-button size="small" @click="browsePath"><template #icon><FolderOpen :size="14" /></template></n-button>
      </div>
    </div>

    <!-- Solid Color -->
    <div v-if="entry.type === 'color'" class="color-row">
      <n-input-number :value="entry.rgb?.[0] ?? 0" size="small" :min="0" :max="255" @update:value="onColorR"><template #prefix>R</template></n-input-number>
      <n-input-number :value="entry.rgb?.[1] ?? 0" size="small" :min="0" :max="255" @update:value="onColorG"><template #prefix>G</template></n-input-number>
      <n-input-number :value="entry.rgb?.[2] ?? 0" size="small" :min="0" :max="255" @update:value="onColorB"><template #prefix>B</template></n-input-number>
      <ColorPicker :model-value="entry.rgb ?? [0,0,0]" @update:model-value="(v: any) => { entry.rgb = v; config.markDirty(); }" />
    </div>

    <!-- Sensor Gauge -->
    <template v-if="entry.type === 'sensor'">
      <div class="field">
        <label class="muted">Sensor source</label>
        <n-select :value="sensorSourceValue()" :options="sensorOptions" size="small" filterable @update:value="onSensorSource" />
      </div>
      <div v-if="entry.sensor?.source?.type === 'command'" class="field">
        <label class="muted">Custom command</label>
        <n-input v-model:value="localCommand" @blur="commitCommand" size="small" />
      </div>
      <SensorGaugeEditor :sensor="ensureSensor()" />
    </template>

    <!-- Custom template -->
    <template v-if="entry.type === 'custom'">
      <div class="template-section">
        <label class="muted">Template</label>
        <!-- Preview thumbnail + dropdown/buttons on the same row (mirrors Slint). -->
        <div class="template-row">
          <div class="template-preview">
            <img v-if="previewJpeg" :src="`data:image/jpeg;base64,${previewJpeg}`" alt="template preview" />
            <div v-else class="preview-ph muted">{{ previewLoading ? "…" : "—" }}</div>
          </div>
          <div class="template-controls">
            <n-select
              :value="entry.template_id ?? ''"
              :options="templateOptions"
              size="small"
              filterable
              @update:value="onTemplateId"
            />
            <div class="template-buttons">
              <n-button size="small" @click="openEditor()">New</n-button>
              <n-button size="small" :disabled="!entry.template_id" @click="entry.template_id && openEditor(entry.template_id)">Edit</n-button>
              <n-button size="small" :disabled="!entry.template_id" @click="duplicateTemplate">Duplicate</n-button>
              <n-button size="small" class="btn-danger" :disabled="!entry.template_id" @click="deleteTemplate">Delete</n-button>
              <n-button size="small" @click="openBrowser">Browse Online</n-button>
            </div>
          </div>
        </div>
        <div class="checkboxes">
          <n-checkbox :checked="entry.smooth_edges ?? false" @update:checked="(v) => { entry.smooth_edges = v; config.markDirty(); }">Smooth edges</n-checkbox>
          <n-checkbox v-if="deviceSupportsH264" :checked="entry.custom_h264 ?? true" @update:checked="(v) => { entry.custom_h264 = v; config.markDirty(); }">H264 streaming</n-checkbox>
          <n-checkbox v-if="supportsCCommand" :checked="entry.aio_512_frame ?? true" @update:checked="(v) => { entry.aio_512_frame = v; config.markDirty(); }">512-byte HID frame</n-checkbox>
        </div>
      </div>
    </template>

    <!-- FPS (video/gif) / update interval (sensor) -->
    <div class="grid">
      <div v-if="['video', 'gif'].includes(entry.type)" class="field">
        <label class="muted">FPS</label>
        <n-input-number :value="entry.fps ?? 30" size="small" :min="1" :max="60" @update:value="onFps" />
      </div>
      <div v-if="entry.type === 'sensor'" class="field">
        <label class="muted">Update interval (ms)</label>
        <n-input-number :value="entry.update_interval_ms ?? 1000" size="small" :min="100" :max="10000" :step="100" @update:value="onUpdateInterval" />
      </div>
    </div>

    <div class="field">
      <label class="muted">Orientation</label>
      <OrientationPicker :model-value="entry.orientation" @update:model-value="onOrientation" />
    </div>

    <div class="field">
      <label class="muted">Brightness</label>
      <LabeledSlider
        :model-value="brightness"
        :min="0"
        :max="100"
        suffix="%"
        @update:model-value="(v: number) => brightness = v"
      />
    </div>
  </div>
</template>

<style scoped>
.lcd-config {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.title {
  font-weight: 600;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--space-3);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.path-row {
  display: flex;
  gap: var(--space-1);
}
.color-row {
  display: flex;
  gap: var(--space-2);
  align-items: flex-end;
}
.template-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
/* Preview thumbnail (left) + dropdown/buttons (right) share one row. */
.template-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
.template-preview {
  width: 96px;
  height: 96px;
  flex-shrink: 0;
  background: #14171f;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}
.template-preview img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.template-preview .preview-ph {
  font-size: var(--font-size-xs);
}
.template-controls {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
/* All template buttons: text-only, same style, equal width. */
.template-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.template-buttons .n-button {
  flex: 1 1 auto;
  min-width: 72px;
}
/* Delete: red hover (Naive UI reads these vars for the hover state). */
.template-buttons :deep(.btn-danger) {
  --n-color-hover: rgba(248, 113, 113, 0.14) !important;
  --n-border-hover: 1px solid var(--danger) !important;
  --n-text-color-hover: var(--danger) !important;
  --n-color-pressed: rgba(248, 113, 113, 0.22) !important;
  --n-border-pressed: 1px solid var(--danger) !important;
  --n-text-color-pressed: var(--danger) !important;
}
.checkboxes {
  display: flex;
  gap: var(--space-4);
  flex-wrap: wrap;
}
</style>
