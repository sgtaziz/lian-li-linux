<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { invoke } from "@tauri-apps/api/core";
import { FolderOpen, Info, Pencil, Sparkles, Trash2 } from "lucide-vue-next";
import type { DeviceInfo, LcdConfig, MediaFraming, MediaType, SensorDescriptor } from "@/types";
import { clampFraming, defaultFraming, isDefaultFraming, MAX_ZOOM, panBy, previewPlacement } from "@/utils/mediaFraming";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdStore } from "@/stores/lcd";
import { useDaemonStore } from "@/stores/daemon";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import { hasSavedLcdDevice, lcdDeviceLabels, lcdEntryKey, resolveLcdDevice } from "@/utils/lcdSelection";
import { useLcdNamesStore } from "@/stores/lcdNames";
import { brightnessError } from "@/utils/brightnessControl";
import { matchesMediaFile, pickMediaFile } from "@/utils/mediaPicker";
import SensorGaugeEditor from "@/components/lcd/SensorGaugeEditor.vue";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import OrientationPicker from "@/components/common/OrientationPicker.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import SensorSelect from "@/components/common/SensorSelect.vue";
import { enumerateSensorsAsOptions, optionForConfig, decodeOption, gaugeTextForSource } from "@/stores/sensorOptions";
import { screenSupportsH264, aio512FrameDefault } from "@/constants/screen";
import StartupImageDialog from "./StartupImageDialog.vue";
import { PIXEL_CLEANER_DURATION_OPTIONS } from "@/constants";

const props = defineProps<{
  entry: LcdConfig;
  index: number;
}>();

const config = useConfigStore();
const devices = useDevicesStore();
const lcd = useLcdStore();
const ipc = useIpc();
const dialog = useDialog();
const message = useMessage();

const lcdDevices = computed(() => devices.lcdDevices);

const lcdNames = useLcdNamesStore();
const deviceLabels = computed(() => lcdDeviceLabels(lcdDevices.value));
const entryKey = computed(() => lcdEntryKey(props.entry));
const customName = computed(() => lcdNames.names[entryKey.value] ?? "");
const deviceLabel = computed(() => {
  const device = deviceForEntry();
  return device ? deviceLabels.value.get(device.device_id) ?? device.name : "";
});
const displayName = computed(() => customName.value || deviceLabel.value || "Screen offline");
const renaming = ref(false);
const nameDraft = ref("");
function startRename() {
  nameDraft.value = customName.value;
  renaming.value = true;
}
function commitName() {
  if (!renaming.value) return;
  lcdNames.setName(entryKey.value, nameDraft.value);
  renaming.value = false;
}
const sharesDevice = computed(() => {
  const device = deviceForEntry();
  if (!device) return false;
  return config.config.lcds.filter((entry) => resolveLcdDevice(entry, lcdDevices.value)?.device_id === device.device_id).length > 1;
});
const needsDeviceChoice = computed(() => !deviceForEntry() || sharesDevice.value);
const deviceOptions = computed(() =>
  [...deviceLabels.value].map(([value, label]) => ({ label, value })),
);

function deviceForEntry(): DeviceInfo | undefined {
  return resolveLcdDevice(props.entry, lcdDevices.value);
}
const selectedDeviceId = computed(
  () => deviceForEntry()?.device_id ?? "",
);
const selectedDevice = computed<DeviceInfo | undefined>(() => deviceForEntry());
const brightnessConfigured = computed(() =>
  hasSavedLcdDevice(selectedDeviceId.value, config.savedLcds, lcdDevices.value),
);

function onSelectDevice(id: string) {
  const d = lcdDevices.value.find((x) => x.device_id === id);
  if (!d) return;
  props.entry.serial = d.device_id;
  props.entry.index = undefined;
  config.markDirty();
}

const mediaTypeOptions = [
  { label: "Image", value: "image" },
  { label: "Video", value: "video" },
  { label: "GIF", value: "gif" },
  { label: "Solid Color", value: "color" },
  { label: "Sensor Gauge", value: "sensor" },
  { label: "Custom Template", value: "custom" },
] as const;

function onMediaType(v: MediaType) {
  const pathKind = v === "sensor" ? "image" : v;
  if ((pathKind === "image" || pathKind === "video" || pathKind === "gif") && props.entry.path && !matchesMediaFile(props.entry.path, pathKind)) {
    props.entry.path = null;
  }
  props.entry.type = v;
  if (v === "sensor") ensureSensor();
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
  const type = props.entry.type === "sensor" ? "image" : props.entry.type;
  if (type !== "image" && type !== "video" && type !== "gif") return;
  try {
    const selected = await pickMediaFile(type);
    if (selected && (props.entry.type === type || props.entry.type === "sensor")) {
      localPath.value = selected;
      commitPath();
    }
  } catch (error) {
    message.error(String(error));
  }
}

// Sensor
const sensorOptions = computed(() => enumerateSensorsAsOptions(config.sensors, true));
function ensureSensor(): SensorDescriptor {
  if (!props.entry.sensor) {
    props.entry.sensor = {
      label: "CPU",
      unit: "%",
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
  const text = gaugeTextForSource(config.sensors, s.source);
  if (text) {
    s.label = text.label;
    s.unit = text.unit;
  }
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

const daemon = useDaemonStore();
const sensorPreview = ref("");
const sensorPreviewError = ref("");
const sensorPreviewSupported = computed(
  () => daemon.connected && (daemon.info?.capabilities.includes("sensor_preview") ?? false),
);
const previewSize = computed(() => {
  const device = selectedDevice.value;
  return device?.screen_width && device?.screen_height
    ? { width: device.screen_width, height: device.screen_height }
    : null;
});
const previewTarget = computed<[number, number]>(() => {
  const size = previewSize.value;
  if (!size) return [1, 1];
  const quarterTurn = Math.round((((props.entry.orientation % 360) + 360) % 360) / 90) % 2 === 1;
  return quarterTurn ? [size.height, size.width] : [size.width, size.height];
});
const previewAspect = computed(() => `${previewTarget.value[0]} / ${previewTarget.value[1]}`);

const entryChanged = computed(
  () => config.dirty && JSON.stringify(config.savedLcds[props.index] ?? null) !== JSON.stringify(props.entry),
);
const applying = ref(false);
async function applyChanges() {
  if (applying.value) return;
  applying.value = true;
  try {
    await config.save();
    message.success("Saved and sent to the LCD");
  } catch (error) {
    message.error(`Save failed: ${error instanceof Error ? error.message : String(error)}`, { duration: 10000 });
  } finally {
    applying.value = false;
  }
}

const FIT_OPTIONS = [
  { label: "Stretch", value: "stretch" },
  { label: "Fit", value: "contain" },
  { label: "Fill", value: "cover" },
];
const framing = computed(() => props.entry.framing ?? defaultFraming());
function setFraming(next: MediaFraming, commit = true) {
  const value = clampFraming(next);
  if (isDefaultFraming(value)) delete props.entry.framing;
  else props.entry.framing = value;
  if (commit) config.markDirty();
}

const sourceSize = ref<[number, number] | null>(null);
function onMediaLoaded(event: Event) {
  const element = event.target as HTMLImageElement | HTMLVideoElement;
  const width = element instanceof HTMLVideoElement ? element.videoWidth : element.naturalWidth;
  const height = element instanceof HTMLVideoElement ? element.videoHeight : element.naturalHeight;
  if (width && height) sourceSize.value = [width, height];
}
const placement = computed(() =>
  sourceSize.value ? previewPlacement(framing.value, sourceSize.value, previewTarget.value) : null,
);
const percentRect = (rect: { x: number; y: number; width: number; height: number }) => ({
  left: `${rect.x}%`,
  top: `${rect.y}%`,
  width: `${rect.width}%`,
  height: `${rect.height}%`,
});

let drag: { x: number; y: number; start: MediaFraming; width: number; height: number } | null = null;
function onPreviewPointerDown(event: PointerEvent) {
  if (!isFileMedia.value || !sourceSize.value || !placement.value) return;
  const element = event.currentTarget as HTMLElement;
  drag = {
    x: event.clientX,
    y: event.clientY,
    start: { ...framing.value },
    width: (element.clientWidth * placement.value.box.width) / 100,
    height: (element.clientHeight * placement.value.box.height) / 100,
  };
  element.setPointerCapture(event.pointerId);
}
function onPreviewPointerMove(event: PointerEvent) {
  if (!drag || !sourceSize.value) return;
  const dx = (event.clientX - drag.x) / Math.max(1, drag.width);
  const dy = (event.clientY - drag.y) / Math.max(1, drag.height);
  setFraming(panBy(drag.start, sourceSize.value, previewTarget.value, dx, dy), false);
}
function onPreviewPointerUp() {
  if (!drag) return;
  drag = null;
  config.markDirty();
}
function onPreviewWheel(event: WheelEvent) {
  if (!isFileMedia.value || !mediaPreviewUrl.value) return;
  event.preventDefault();
  setFraming({ ...framing.value, zoom: framing.value.zoom * Math.exp(-event.deltaY * 0.0015) });
}
const showSensorPreview = computed(
  () => props.entry.type === "sensor" && sensorPreviewSupported.value && !!previewSize.value,
);
const isFileMedia = computed(() => ["image", "gif", "video"].includes(props.entry.type));
const showPreview = computed(
  () => !!previewSize.value && (showSensorPreview.value || isFileMedia.value || props.entry.type === "color"),
);

const MEDIA_MIME: Record<string, string> = {
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", bmp: "image/bmp", gif: "image/gif",
  mp4: "video/mp4", m4v: "video/mp4", webm: "video/webm", mov: "video/quicktime",
  mkv: "video/x-matroska", avi: "video/x-msvideo",
};
const mediaPreviewUrl = ref("");
const mediaPreviewError = ref("");
let mediaPreviewRevision = 0;

function clearMediaPreview() {
  if (mediaPreviewUrl.value) URL.revokeObjectURL(mediaPreviewUrl.value);
  mediaPreviewUrl.value = "";
  sourceSize.value = null;
}

watch(
  () => [isFileMedia.value && showPreview.value, props.entry.path] as const,
  async ([visible, path]) => {
    const revision = ++mediaPreviewRevision;
    clearMediaPreview();
    mediaPreviewError.value = "";
    if (!visible || !path) return;
    try {
      const bytes = await invoke<ArrayBuffer>("media_preview", { path });
      if (revision !== mediaPreviewRevision) return;
      const extension = path.split(".").pop()?.toLowerCase() ?? "";
      mediaPreviewUrl.value = URL.createObjectURL(new Blob([bytes], { type: MEDIA_MIME[extension] ?? "" }));
    } catch (error) {
      if (revision === mediaPreviewRevision) mediaPreviewError.value = String(error);
    }
  },
  { immediate: true },
);
onUnmounted(() => {
  mediaPreviewRevision++;
  clearMediaPreview();
});

const previewError = computed(() => {
  if (props.entry.type === "sensor") return sensorPreviewError.value;
  return isFileMedia.value ? mediaPreviewError.value : "";
});
const previewCaption = computed(() => {
  const size = previewSize.value ? `${previewSize.value.width}×${previewSize.value.height}` : "";
  if (previewError.value) return "Preview unavailable";
  if (props.entry.type === "sensor") return `${size} · live, unsaved`;
  if (isFileMedia.value) {
    if (!props.entry.path) return `${size} · no file selected`;
    const fit = FIT_OPTIONS.find((option) => option.value === framing.value.fit)?.label ?? "";
    const zoom = framing.value.zoom > 1 ? ` · ${framing.value.zoom.toFixed(1)}×` : "";
    return `${size} · ${fit}${zoom}`;
  }
  return size;
});
let previewInFlight = false;
let previewQueued = false;

async function refreshSensorPreview() {
  if (!showSensorPreview.value || !props.entry.sensor) {
    sensorPreview.value = "";
    return;
  }
  if (previewInFlight) {
    previewQueued = true;
    return;
  }
  previewInFlight = true;
  try {
    const res = await ipc.request<{ jpeg_base64: string }>("RenderSensorPreview", {
      lcd: JSON.parse(JSON.stringify(props.entry)),
      ...previewSize.value,
    });
    sensorPreview.value = res.jpeg_base64 ?? "";
    sensorPreviewError.value = "";
  } catch (error) {
    sensorPreviewError.value = String(error);
  } finally {
    previewInFlight = false;
    if (previewQueued) {
      previewQueued = false;
      void refreshSensorPreview();
    }
  }
}

const scheduleSensorPreview = useDebounce(() => void refreshSensorPreview(), 300);
watch(
  () => [
    showSensorPreview.value,
    JSON.stringify(props.entry.sensor),
    props.entry.path,
    props.entry.orientation,
    previewSize.value?.width,
    previewSize.value?.height,
  ],
  () => scheduleSensorPreview(),
  { immediate: true },
);

let livePreviewTimer: ReturnType<typeof setInterval> | undefined;
watch(
  () => [showSensorPreview.value, props.entry.update_interval_ms] as const,
  ([visible, interval]) => {
    clearInterval(livePreviewTimer);
    livePreviewTimer = undefined;
    if (!visible) return;
    livePreviewTimer = setInterval(() => {
      if (!document.hidden && document.hasFocus()) void refreshSensorPreview();
    }, Math.max(1000, interval ?? 1000));
  },
  { immediate: true },
);
function onWindowFocus() {
  if (showSensorPreview.value) void refreshSensorPreview();
}
window.addEventListener("focus", onWindowFocus);
onUnmounted(() => {
  clearInterval(livePreviewTimer);
  window.removeEventListener("focus", onWindowFocus);
});

// Custom template sub-section
const templateOptions = computed(() =>
  config.templates.map((t) => ({ label: t.name, value: t.id })),
);
const deviceSupportsH264 = computed(() =>
  screenSupportsH264(selectedDevice.value?.family ?? ("Ene6k77" as any)),
);
const supportsCCommand = computed(() => selectedDevice.value?.supports_c_command ?? false);
const aio512Default = computed(() =>
  selectedDevice.value ? aio512FrameDefault(selectedDevice.value.family) : true,
);

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
    content: `${displayName.value} settings will be removed from the configuration.`,
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
    if (selectedDeviceId.value && brightnessConfigured.value) {
      lcd.setBrightness(selectedDeviceId.value, v);
    }
  },
});

const currentBrightnessError = computed(() => brightnessError(
  brightness.value,
  devices.telemetry.lcd_brightness?.[selectedDeviceId.value],
  lcd.brightnessErrors[selectedDeviceId.value],
  lcd.brightnessRequests[selectedDeviceId.value],
));

const cleanerDurationOptions = PIXEL_CLEANER_DURATION_OPTIONS.map((opt) => ({
  label: opt.label,
  key: opt.value,
}));

const cleanerTargetId = computed(() => {
  const devId = selectedDeviceId.value || props.entry.serial || "";
  return devId ? `${devId}#${props.index}` : `${props.index}`;
});

const isCleaningThis = computed(() => {
  return lcd.isCleaning(cleanerTargetId.value, props.index);
});

const remainingFormatted = computed(() => {
  return lcd.formattedRemainingFor(cleanerTargetId.value, props.index);
});

const isPreparingThis = computed(() => lcd.isPreparing(cleanerTargetId.value));
const stopPending = ref(false);
const settingsLocked = computed(() => isCleaningThis.value || isPreparingThis.value || stopPending.value);

async function handleStartClean(key: string | number) {
  const minutes = Number(key);
  try {
    const result = await lcd.startPixelClean(cleanerTargetId.value, minutes);
    if (result.started) message.success(`Pixel cleaner started for ${minutes} minutes at 75% brightness`);
    else if (result.cancelled) message.info("Pixel cleaner preparation cancelled");
  } catch (err) {
    message.error(`Failed to start pixel cleaner: ${err}`);
  }
}

async function handleStopClean() {
  stopPending.value = true;
  try {
    const result = await lcd.stopPixelClean(cleanerTargetId.value);
    if (result.stopped) message.info("Pixel cleaner stopped. Previous display restoration requested");
    else message.warning("This session was not stopped. Refreshed its status from the daemon");
  } catch (err) {
    message.error(`Failed to stop pixel cleaner: ${err}`);
  } finally {
    stopPending.value = false;
  }
}

</script>

<template>
  <div class="card lcd-config">
    <div class="head">
      <div class="title-wrap">
        <n-input
          v-if="renaming"
          v-model:value="nameDraft"
          size="small"
          class="name-input"
          :placeholder="deviceLabel || 'Screen name'"
          :maxlength="40"
          autofocus
          @blur="commitName"
          @keydown.enter="commitName"
          @keydown.esc="renaming = false"
        />
        <template v-else>
          <span class="title">{{ displayName }}</span>
          <button class="rename-btn" title="Rename this screen" @click="startRename"><Pencil :size="12" /></button>
          <span v-if="customName && deviceLabel" class="device-label">{{ deviceLabel }}</span>
        </template>
      </div>
      <div class="head-actions">
        <template v-if="entryChanged">
          <span class="unsaved">Unsaved changes</span>
          <n-button
            size="small"
            type="primary"
            :loading="applying"
            :disabled="!daemon.canWrite || settingsLocked"
            title="Saves every pending change in the app, including other pages, and sends this LCD its new settings"
            @click="applyChanges"
          >Save all</n-button>
        </template>
        <StartupImageDialog v-if="selectedDevice?.startup_image" :device="selectedDevice" />
        <n-button
          v-if="isPreparingThis"
          size="small"
          quaternary
          title="Cancel pixel cleaner preparation"
          @click="lcd.cancelPixelPreparation(cleanerTargetId)"
        >Cancel</n-button>
        <n-button
          v-else-if="isCleaningThis"
          :loading="stopPending"
          size="small"
          quaternary
          type="warning"
          class="cleaner-btn cleaner-btn-running"
          @click="handleStopClean"
          :title="`Stop pixel conditioning (${remainingFormatted} remaining)`"
        >
          <template #icon><Sparkles :size="14" /></template>
        </n-button>
        <n-dropdown
          v-else
          trigger="click"
          placement="bottom-end"
          :options="cleanerDurationOptions"
          :disabled="!selectedDeviceId || lcd.preparingCleaner || stopPending"
          @select="handleStartClean"
        >
          <n-button
            size="small"
            quaternary
            type="warning"
            class="cleaner-btn"
            :disabled="!selectedDeviceId || lcd.preparingCleaner || stopPending"
            title="Run pixel conditioning to clear image retention"
          >
            <template #icon><Sparkles :size="14" /></template>
          </n-button>
        </n-dropdown>
        <n-button
          size="small"
          quaternary
          type="error"
          @click="removeEntry"
          :disabled="settingsLocked"
          title="Delete LCD configuration"
        >
          <template #icon><Trash2 :size="14" /></template>
        </n-button>
      </div>
    </div>

    <div v-if="isPreparingThis" class="card-cleaner-notice">Preparing pixel cleaner…</div>
    <div v-if="isCleaningThis" class="card-cleaner-notice">
      <Info :size="14" class="cleaner-notice-icon" />
      <span>
        Pixel conditioning in progress (<strong>{{ remainingFormatted }}</strong> remaining at 75% brightness). Display settings locked during conditioning.
      </span>
    </div>

    <div class="card-content" :inert="settingsLocked" :class="{ 'card-body-locked': settingsLocked }">
      <n-alert v-if="needsDeviceChoice" type="warning" :show-icon="false" class="assign-alert">
        <div class="field">
          <label>{{ selectedDevice ? "Another entry also drives this screen. Pick the screen this entry belongs to." : "This screen is not connected. Assign the entry to a connected screen." }}</label>
          <n-select
            :value="selectedDeviceId || null"
            :options="deviceOptions"
            size="small"
            placeholder="Assign to screen"
            @update:value="onSelectDevice"
          />
        </div>
      </n-alert>
      <div class="grid">
      <div class="field">
        <label class="muted">Media type</label>
        <n-select :value="entry.type" :options="mediaTypeOptions" size="small" @update:value="onMediaType" />
      </div>
      <div v-if="['video', 'gif'].includes(entry.type)" class="field">
        <label class="muted">FPS</label>
        <n-input-number :value="entry.fps ?? 30" size="small" :min="1" :max="120" @update:value="onFps" />
      </div>
      <div v-if="entry.type === 'sensor'" class="field">
        <label class="muted">Update interval (ms)</label>
        <n-input-number :value="entry.update_interval_ms ?? 1000" size="small" :min="100" :max="10000" :step="100" @update:value="onUpdateInterval" />
      </div>
    </div>

    <div class="screen-row">
      <div class="field">
        <label class="muted">Orientation</label>
        <OrientationPicker :model-value="entry.orientation" @update:model-value="onOrientation" />
      </div>
      <div class="field brightness">
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
    <n-alert v-if="currentBrightnessError" type="error">
      Could not change screen brightness: {{ currentBrightnessError }}
    </n-alert>
    <p v-if="selectedDeviceId && !brightnessConfigured" class="hint">
      Save this LCD configuration to apply brightness.
    </p>

    <div class="divider" />

    <div v-if="entry.type !== 'custom'" class="media-layout" :class="{ 'with-preview': showPreview }">
      <div class="media-settings">
        <div v-if="isFileMedia" class="field">
          <label class="muted">Path</label>
          <div class="path-row">
            <n-input v-model:value="localPath" @blur="commitPath" size="small" placeholder="/path/to/media" />
            <n-button size="small" @click="browsePath"><template #icon><FolderOpen :size="14" /></template></n-button>
          </div>
        </div>
        <div v-if="isFileMedia" class="framing-row">
          <div class="field">
            <label class="muted">Fit</label>
            <n-radio-group :value="framing.fit" size="small" @update:value="(fit) => setFraming({ ...framing, fit })">
              <n-radio-button v-for="option in FIT_OPTIONS" :key="option.value" :value="option.value">{{ option.label }}</n-radio-button>
            </n-radio-group>
          </div>
          <div class="field zoom">
            <label class="muted">Zoom</label>
            <LabeledSlider
              :model-value="framing.zoom"
              :min="1"
              :max="MAX_ZOOM"
              :step="0.05"
              :format="(v: number) => `${v.toFixed(2)}×`"
              @update:model-value="(zoom: number) => setFraming({ ...framing, zoom })"
            />
          </div>
          <n-button size="small" quaternary :disabled="isDefaultFraming(entry.framing)" @click="setFraming(defaultFraming())">Reset</n-button>
        </div>
        <p v-if="isFileMedia && mediaPreviewUrl" class="hint">Drag the preview to move the image, scroll on it to zoom.</p>

        <div v-if="entry.type === 'color'" class="field">
          <label class="muted">Color</label>
          <ColorPicker :model-value="entry.rgb ?? [0,0,0]" @update:model-value="(v: any) => { entry.rgb = v; config.markDirty(); }" />
        </div>

        <template v-if="entry.type === 'sensor'">
          <div class="grid">
            <div class="field">
              <label class="muted">Sensor source</label>
              <SensorSelect :value="sensorSourceValue()" :options="sensorOptions" size="small" filterable @update:value="onSensorSource" />
            </div>
            <div class="field">
              <label class="muted">Background image</label>
              <div class="path-row">
                <n-input v-model:value="localPath" @blur="commitPath" size="small" placeholder="None (solid color)" clearable @clear="localPath = ''; commitPath()" />
                <n-button size="small" @click="browsePath"><template #icon><FolderOpen :size="14" /></template></n-button>
              </div>
            </div>
          </div>
          <div v-if="entry.sensor?.source?.type === 'command'" class="field">
            <label class="muted">Custom command</label>
            <n-input v-model:value="localCommand" @blur="commitCommand" size="small" />
          </div>
          <SensorGaugeEditor v-if="entry.sensor" :sensor="entry.sensor" />
        </template>
      </div>

      <aside v-if="showPreview" class="media-preview">
        <div
          class="preview-frame"
          :class="{ pannable: isFileMedia && !!mediaPreviewUrl }"
          :style="{ aspectRatio: previewAspect, '--preview-brightness': brightness / 100 }"
          @pointerdown="onPreviewPointerDown"
          @pointermove="onPreviewPointerMove"
          @pointerup="onPreviewPointerUp"
          @pointercancel="onPreviewPointerUp"
          @wheel="onPreviewWheel"
        >
          <img v-if="entry.type === 'sensor' && sensorPreview" :src="`data:image/jpeg;base64,${sensorPreview}`" alt="Sensor gauge preview" />
          <div v-else-if="isFileMedia && mediaPreviewUrl" class="framing-box" :style="placement ? percentRect(placement.box) : undefined">
            <video
              v-if="entry.type === 'video'"
              class="framed-media"
              :src="mediaPreviewUrl"
              :style="placement ? percentRect(placement.media) : undefined"
              autoplay
              muted
              loop
              playsinline
              @loadedmetadata="onMediaLoaded"
              @error="mediaPreviewError = 'This video format cannot be previewed here'"
            />
            <img
              v-else
              class="framed-media"
              :src="mediaPreviewUrl"
              :style="placement ? percentRect(placement.media) : undefined"
              alt="Media preview"
              draggable="false"
              @load="onMediaLoaded"
            />
          </div>
          <div v-else-if="entry.type === 'color'" class="preview-color" :style="{ background: `rgb(${(entry.rgb ?? [0, 0, 0]).join(',')})` }" />
          <span v-else class="muted">{{ previewError ? "!" : "…" }}</span>
        </div>
        <span class="preview-caption" :class="{ error: previewError }" :title="previewError">{{ previewCaption }}</span>
      </aside>
    </div>

    <!-- Custom template -->
    <template v-if="entry.type === 'custom'">
      <div class="template-section">
        <label class="muted">Template</label>
        <!-- Preview thumbnail + dropdown/buttons on the same row (mirrors Slint). -->
        <div class="template-row">
          <div class="template-preview" :style="{ '--preview-brightness': brightness / 100 }">
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
        </div>
      </div>
    </template>

    <div v-if="supportsCCommand" class="checkboxes">
      <n-checkbox :checked="entry.aio_512_frame ?? aio512Default" @update:checked="(v) => { entry.aio_512_frame = v; config.markDirty(); }">512-byte HID frame</n-checkbox>
    </div>

    </div>
  </div>
</template>

<style scoped>
.lcd-config {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.title {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.title-wrap {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.name-input {
  width: 220px;
}
.rename-btn {
  display: inline-flex;
  padding: 2px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-muted);
  cursor: pointer;
}
.rename-btn:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}
.device-label {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  white-space: nowrap;
}
.assign-alert .field label {
  font-size: var(--font-size-sm);
}
.head-actions {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
.unsaved {
  margin-right: var(--space-1);
  font-size: var(--font-size-xs);
  color: var(--warning);
}
.cleaner-btn {
  color: var(--warning);
  transition: filter 0.2s ease, transform 0.15s ease;
}
.cleaner-btn:hover:not(:disabled) {
  filter: drop-shadow(0 0 5px rgba(251, 191, 36, 0.55));
}
.cleaner-btn-running {
  color: var(--warning);
  animation: pulse-glow 2s infinite ease-in-out;
}
@keyframes pulse-glow {
  0%,
  100% {
    filter: drop-shadow(0 0 2px rgba(251, 191, 36, 0.4));
    opacity: 0.85;
  }
  50% {
    filter: drop-shadow(0 0 8px rgba(251, 191, 36, 0.85));
    opacity: 1;
  }
}
.card-cleaner-notice {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-sm);
  background: var(--accent-soft);
  border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  font-size: var(--font-size-xs);
  color: var(--accent);
}
.cleaner-notice-icon {
  flex-shrink: 0;
  color: var(--accent);
}
.card-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  transition: opacity 0.2s ease;
}
.card-body-locked {
  pointer-events: none;
  opacity: 0.45;
  user-select: none;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
  gap: var(--space-2) var(--space-3);
}
.screen-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-2) var(--space-4);
}
.screen-row .brightness {
  flex: 1;
  min-width: 180px;
}
.divider {
  border-top: 1px solid var(--border);
}
.media-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: var(--space-4);
  align-items: start;
}
.media-layout.with-preview {
  grid-template-columns: minmax(0, 1fr) 200px;
}
.media-settings {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.media-preview {
  position: sticky;
  top: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-1);
}
.framing-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-2) var(--space-4);
}
.framing-row .zoom {
  flex: 1;
  min-width: 160px;
}
.framing-box {
  position: absolute;
  inset: 0;
  overflow: hidden;
}
.preview-frame .framed-media {
  position: absolute;
  left: 0;
  top: 0;
  max-width: none;
  user-select: none;
  pointer-events: none;
}
.preview-frame.pannable {
  cursor: grab;
  touch-action: none;
}
.preview-frame.pannable:active {
  cursor: grabbing;
}
.preview-frame > * {
  filter: brightness(var(--preview-brightness, 1));
}
.preview-frame {
  position: relative;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  background: #000;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.preview-frame img,
.preview-frame video,
.preview-color {
  width: 100%;
  height: 100%;
  object-fit: fill;
}
.preview-caption {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.preview-caption.error {
  color: var(--warning);
  cursor: help;
}
@media (max-width: 760px) {
  .media-layout.with-preview {
    grid-template-columns: minmax(0, 1fr);
  }
  .media-preview {
    position: static;
    order: -1;
    width: 200px;
    justify-self: center;
  }
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
  filter: brightness(var(--preview-brightness, 1));
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
