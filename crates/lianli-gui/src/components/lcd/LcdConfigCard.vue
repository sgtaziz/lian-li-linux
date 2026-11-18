<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { backendText, invoke } from "@/utils/backendText";
import { FolderOpen, Info, Pause, Play, Sparkles, Trash2 } from "lucide-vue-next";
import type { DeviceInfo, LcdConfig, MediaFraming, MediaType, SensorDescriptor } from "@/types";
import { clampFraming, defaultFraming, isDefaultFraming, MAX_ZOOM, panBy, previewPlacement } from "@/utils/mediaFraming";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdStore } from "@/stores/lcd";
import { useDaemonStore } from "@/stores/daemon";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import { hasSavedLcdDevice, lcdEntryKey, resolveLcdDevice } from "@/utils/lcdSelection";
import { useLcdScreensStore } from "@/stores/lcdScreens";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import EditableName from "@/components/common/EditableName.vue";
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
import { useTranslate, type MessageKey } from "@/i18n";

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
const t = useTranslate();

const lcdDevices = computed(() => devices.lcdDevices);

const screens = useLcdScreensStore();
const names = useDeviceNamesStore();
const labels = computed(() => devices.labels);
const entryKey = computed(() => lcdEntryKey(props.entry));
const customName = computed(() => names.lcdEntryName(props.entry));
const deviceLabel = computed(() => {
  const device = deviceForEntry();
  return device ? labels.value.get(device.device_id) ?? device.name : "";
});
const displayName = computed(() => customName.value || deviceLabel.value || t("lcd.screenOffline"));
const sharesDevice = computed(() => {
  const device = deviceForEntry();
  if (!device) return false;
  return config.config.lcds.filter((entry) => resolveLcdDevice(entry, lcdDevices.value)?.device_id === device.device_id).length > 1;
});
const needsDeviceChoice = computed(() => !deviceForEntry() || sharesDevice.value);
const deviceOptions = computed(() =>
  [...lcdDevices.value].sort((a, b) => a.device_id.localeCompare(b.device_id)).map((device) => ({
    label: names.label(device.device_id, labels.value.get(device.device_id) ?? device.name),
    value: device.device_id,
  })),
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
  const previousKey = entryKey.value;
  const previousName = customName.value;
  props.entry.serial = d.device_id;
  props.entry.index = undefined;
  screens.carryOver(previousKey, entryKey.value);
  if (previousName && !names.nameOf(d.device_id)) names.setName(d.device_id, previousName);
  config.markDirty();
}

const mediaTypeOptions = computed(() => [
  { label: t("lcd.mediaTypes.image"), value: "image" },
  { label: t("lcd.mediaTypes.video"), value: "video" },
  { label: t("lcd.mediaTypes.gif"), value: "gif" },
  { label: t("lcd.configCard.mediaTypeOptions.solidColor"), value: "color" },
  { label: t("lcd.configCard.mediaTypeOptions.sensorGauge"), value: "sensor" },
  { label: t("lcd.configCard.mediaTypeOptions.customTemplate"), value: "custom" },
]);

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
const previewSize = computed<{ width: number; height: number } | null>((previous) => {
  const device = selectedDevice.value;
  if (!device?.screen_width || !device?.screen_height) return null;
  if (previous?.width === device.screen_width && previous.height === device.screen_height) return previous;
  return { width: device.screen_width, height: device.screen_height };
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
    message.success(t("lcd.configCard.saved"));
  } catch (error) {
    message.error(t("lcd.configCard.saveFailed", { error: error instanceof Error ? error.message : String(error) }), { duration: 10000 });
  } finally {
    applying.value = false;
  }
}

const FIT_OPTIONS = computed(() => [
  { label: t("lcd.configCard.fitOptions.stretch"), value: "stretch" },
  { label: t("lcd.configCard.fitOptions.contain"), value: "contain" },
  { label: t("lcd.configCard.fitOptions.cover"), value: "cover" },
]);
const framing = computed(() => props.entry.framing ?? defaultFraming());
function setFraming(next: MediaFraming, commit = true) {
  const value = clampFraming(next);
  if (isDefaultFraming(value)) delete props.entry.framing;
  else props.entry.framing = value;
  if (commit) config.markDirty();
}

const sourceSize = ref<[number, number] | null>(null);
const previewVideo = ref<HTMLVideoElement | null>(null);
const previewPlaying = ref(false);
function togglePreviewPlayback() {
  const video = previewVideo.value;
  if (!video) return;
  if (video.paused) void video.play().catch(() => undefined);
  else video.pause();
}

function onMediaLoaded(event: Event) {
  const element = event.target as HTMLImageElement | HTMLVideoElement;
  const width = element instanceof HTMLVideoElement ? element.videoWidth : element.naturalWidth;
  const height = element instanceof HTMLVideoElement ? element.videoHeight : element.naturalHeight;
  if (width && height) sourceSize.value = [width, height];
  if (element instanceof HTMLVideoElement && element.paused) element.currentTime = 0.001;
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
  previewPlaying.value = false;
  sourceSize.value = null;
}

watch(
  [() => isFileMedia.value && showPreview.value, () => props.entry.path],
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
  if (previewError.value) return t("lcd.configCard.previewUnavailable");
  if (props.entry.type === "sensor") return t("lcd.configCard.captionLive", { size });
  if (isFileMedia.value) {
    if (!props.entry.path) return t("lcd.configCard.captionNoFile", { size });
    const fit = FIT_OPTIONS.value.find((option) => option.value === framing.value.fit)?.label ?? "";
    if (framing.value.zoom > 1) return t("lcd.configCard.captionFitZoom", { size, fit, zoom: framing.value.zoom.toFixed(1) });
    return t("lcd.configCard.captionFit", { size, fit });
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
  config.markDirty();
  await config.load({ keepDraft: true });
  renderPreview();
}

/** Delete the entry's currently-selected template (clears all references). */
async function deleteTemplate() {
  const id = props.entry.template_id;
  if (!id) return;
  dialog.error({
    title: t("lcd.configCard.deleteTemplateTitle"),
    content: t("lcd.configCard.deleteTemplateContent"),
    positiveText: t("common.delete"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      config.templates = config.templates.filter((t) => t.id !== id);
      for (const e of config.config.lcds) {
        if (e.template_id === id) e.template_id = null;
      }
      await lcd.setTemplates(config.templates);
      config.markDirty();
      await config.load({ keepDraft: true });
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
    title: t("lcd.configCard.removeEntryTitle"),
    content: t("lcd.configCard.removeEntryContent", { name: displayName.value }),
    positiveText: t("common.remove"),
    negativeText: t("common.cancel"),
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

const currentBrightnessStatus = computed(() => {
  const status = devices.telemetry.lcd_brightness?.[selectedDeviceId.value];
  return status && { ...status, error: status.error && backendText(status.error, status.error_ui) };
});

const currentBrightnessError = computed(() => brightnessError(
  brightness.value,
  currentBrightnessStatus.value,
  lcd.brightnessErrors[selectedDeviceId.value],
  lcd.brightnessRequests[selectedDeviceId.value],
));

const cleanerDurationOptions = computed(() => PIXEL_CLEANER_DURATION_OPTIONS.map((opt) => ({
  label: t(opt.labelKey),
  key: opt.value,
})));

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

const cleanerProgressKey: MessageKey = "lcd.cleaner.inProgress";
const isPreparingThis = computed(() => lcd.isPreparing(cleanerTargetId.value));
const stopPending = ref(false);
const settingsLocked = computed(() => isCleaningThis.value || isPreparingThis.value || stopPending.value);

async function handleStartClean(key: string | number) {
  const minutes = Number(key);
  try {
    const result = await lcd.startPixelClean(cleanerTargetId.value, minutes);
    if (result.started) message.success(t("lcd.cleaner.started", { minutes }));
    else if (result.cancelled) message.info(t("lcd.cleaner.cancelled"));
  } catch (err) {
    message.error(t("lcd.cleaner.startFailed", { error: String(err) }));
  }
}

async function handleStopClean() {
  stopPending.value = true;
  try {
    const result = await lcd.stopPixelClean(cleanerTargetId.value);
    if (result.stopped) message.info(t("lcd.cleaner.stopped"));
    else message.warning(t("lcd.cleaner.notStopped"));
  } catch (err) {
    message.error(t("lcd.cleaner.stopFailed", { error: String(err) }));
  } finally {
    stopPending.value = false;
  }
}

</script>

<template>
  <div class="card lcd-config">
    <div class="head">
      <EditableName
        :name="customName"
        :label="deviceLabel"
        :empty-label="t('lcd.screenOffline')"
        :rename-title="t('lcd.configCard.renameTitle')"
        @rename="(name) => names.setLcdEntryName(entry, name)"
      />
      <div class="head-actions">
        <template v-if="entryChanged">
          <span class="unsaved">{{ t("lcd.configCard.unsaved") }}</span>
          <n-button
            size="small"
            type="primary"
            :loading="applying"
            :disabled="!daemon.canWrite || settingsLocked"
            :title="t('lcd.configCard.saveAllTitle')"
            @click="applyChanges"
          >{{ t("lcd.configCard.saveAll") }}</n-button>
        </template>
        <StartupImageDialog v-if="selectedDevice?.startup_image" :device="selectedDevice" />
        <n-button
          v-if="isPreparingThis"
          size="small"
          quaternary
          :title="t('lcd.cleaner.cancelPreparationTitle')"
          @click="lcd.cancelPixelPreparation(cleanerTargetId)"
        >{{ t("common.cancel") }}</n-button>
        <n-button
          v-else-if="isCleaningThis"
          :loading="stopPending"
          size="small"
          quaternary
          type="warning"
          class="cleaner-btn cleaner-btn-running"
          @click="handleStopClean"
          :title="t('lcd.cleaner.stopTitle', { remaining: remainingFormatted })"
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
            :title="t('lcd.cleaner.runTitle')"
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
          :title="t('lcd.configCard.deleteTitle')"
        >
          <template #icon><Trash2 :size="14" /></template>
        </n-button>
      </div>
    </div>

    <div v-if="isPreparingThis" class="card-cleaner-notice">{{ t("lcd.cleaner.preparing") }}</div>
    <div v-if="isCleaningThis" class="card-cleaner-notice">
      <Info :size="14" class="cleaner-notice-icon" />
      <i18n-t :keypath="cleanerProgressKey" tag="span" scope="global">
        <template #remaining><strong>{{ remainingFormatted }}</strong></template>
      </i18n-t>
    </div>

    <div class="card-content" :inert="settingsLocked" :class="{ 'card-body-locked': settingsLocked }">
      <n-alert v-if="needsDeviceChoice" type="warning" :show-icon="false" class="assign-alert">
        <div class="field">
          <label>{{ selectedDevice ? t("lcd.configCard.assignShared") : t("lcd.configCard.assignOffline") }}</label>
          <n-select
            :value="selectedDeviceId || null"
            :options="deviceOptions"
            size="small"
            :placeholder="t('lcd.configCard.assignPlaceholder')"
            @update:value="onSelectDevice"
          />
        </div>
      </n-alert>
    <section class="panel">
      <h3 class="panel-title">{{ t("lcd.configCard.screenSection") }}</h3>
      <div class="grid">
      <div class="field">
        <label class="muted">{{ t("lcd.configCard.mediaType") }}</label>
        <n-select :value="entry.type" :options="mediaTypeOptions" size="small" @update:value="onMediaType" />
      </div>
      <div v-if="['video', 'gif'].includes(entry.type)" class="field">
        <label class="muted">{{ t("lcd.configCard.fps") }}</label>
        <n-input-number :value="entry.fps ?? 30" size="small" :min="1" :max="120" @update:value="onFps" />
      </div>
      <div v-if="entry.type === 'sensor'" class="field">
        <label class="muted">{{ t("lcd.configCard.updateInterval") }}</label>
        <n-input-number :value="entry.update_interval_ms ?? 1000" size="small" :min="100" :max="10000" :step="100" @update:value="onUpdateInterval" />
      </div>
    </div>

    <div class="screen-row">
      <div class="field">
        <label class="muted">{{ t("lcd.configCard.orientation") }}</label>
        <OrientationPicker :model-value="entry.orientation" @update:model-value="onOrientation" />
      </div>
      <div class="field brightness">
        <label class="muted">{{ t("lcd.configCard.brightness") }}</label>
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
      {{ t("lcd.configCard.brightnessError", { error: currentBrightnessError }) }}
    </n-alert>
    <p v-if="selectedDeviceId && !brightnessConfigured" class="hint">
      {{ t("lcd.configCard.saveForBrightness") }}
    </p>
    </section>

    <section class="panel">
    <h3 class="panel-title">{{ t("lcd.configCard.contentSection") }}</h3>
    <div v-if="entry.type !== 'custom'" class="media-layout" :class="{ 'with-preview': showPreview }">
      <div class="media-settings">
        <div v-if="isFileMedia" class="field">
          <label class="muted">{{ t("lcd.configCard.path") }}</label>
          <div class="path-row">
            <n-input v-model:value="localPath" @blur="commitPath" size="small" placeholder="/path/to/media" />
            <n-button size="small" @click="browsePath"><template #icon><FolderOpen :size="14" /></template></n-button>
          </div>
        </div>
        <div v-if="isFileMedia" class="framing-row">
          <div class="field">
            <label class="muted">{{ t("lcd.configCard.fit") }}</label>
            <n-radio-group :value="framing.fit" size="small" @update:value="(fit) => setFraming({ ...framing, fit })">
              <n-radio-button v-for="option in FIT_OPTIONS" :key="option.value" :value="option.value">{{ option.label }}</n-radio-button>
            </n-radio-group>
          </div>
          <div class="field zoom">
            <label class="muted">{{ t("lcd.configCard.zoom") }}</label>
            <LabeledSlider
              :model-value="framing.zoom"
              :min="1"
              :max="MAX_ZOOM"
              :step="0.05"
              :format="(v: number) => `${v.toFixed(2)}×`"
              @update:model-value="(zoom: number) => setFraming({ ...framing, zoom })"
            />
          </div>
          <n-button size="small" quaternary :disabled="isDefaultFraming(entry.framing)" @click="setFraming(defaultFraming())">{{ t("common.reset") }}</n-button>
        </div>
        <p v-if="isFileMedia && mediaPreviewUrl" class="hint">{{ t("lcd.configCard.dragHint") }}</p>

        <div v-if="entry.type === 'color'" class="field">
          <label class="muted">{{ t("lcd.configCard.color") }}</label>
          <ColorPicker :model-value="entry.rgb ?? [0,0,0]" @update:model-value="(v: any) => { entry.rgb = v; config.markDirty(); }" />
        </div>

        <template v-if="entry.type === 'sensor'">
          <div class="grid">
            <div class="field">
              <label class="muted">{{ t("lcd.configCard.sensorSource") }}</label>
              <SensorSelect :value="sensorSourceValue()" :options="sensorOptions" size="small" filterable @update:value="onSensorSource" />
            </div>
            <div class="field">
              <label class="muted">{{ t("lcd.configCard.backgroundImage") }}</label>
              <div class="path-row">
                <n-input v-model:value="localPath" @blur="commitPath" size="small" :placeholder="t('lcd.configCard.backgroundPlaceholder')" clearable @clear="localPath = ''; commitPath()" />
                <n-button size="small" @click="browsePath"><template #icon><FolderOpen :size="14" /></template></n-button>
              </div>
            </div>
          </div>
          <div v-if="entry.sensor?.source?.type === 'command'" class="field">
            <label class="muted">{{ t("lcd.configCard.customCommand") }}</label>
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
          <img v-if="entry.type === 'sensor' && sensorPreview" :src="`data:image/jpeg;base64,${sensorPreview}`" :alt="t('lcd.configCard.sensorPreviewAlt')" />
          <div v-else-if="isFileMedia && mediaPreviewUrl" class="framing-box" :style="placement ? percentRect(placement.box) : undefined">
            <video
              v-if="entry.type === 'video'"
              ref="previewVideo"
              class="framed-media"
              :src="mediaPreviewUrl"
              :style="placement ? percentRect(placement.media) : undefined"
              preload="auto"
              muted
              loop
              playsinline
              @play="previewPlaying = true"
              @pause="previewPlaying = false"
              @loadedmetadata="onMediaLoaded"
              @error="mediaPreviewError = t('lcd.configCard.videoUnsupported')"
            />
            <img
              v-else
              class="framed-media"
              :src="mediaPreviewUrl"
              :style="placement ? percentRect(placement.media) : undefined"
              :alt="t('lcd.configCard.mediaPreviewAlt')"
              draggable="false"
              @load="onMediaLoaded"
            />
          </div>
          <div v-else-if="entry.type === 'color'" class="preview-color" :style="{ background: `rgb(${(entry.rgb ?? [0, 0, 0]).join(',')})` }" />
          <span v-else class="muted">{{ previewError ? "!" : "…" }}</span>
        </div>
        <div class="preview-footer">
          <span class="preview-caption" :class="{ error: previewError }" :title="previewError">{{ previewCaption }}</span>
          <button
            v-if="entry.type === 'video' && mediaPreviewUrl"
            class="preview-play"
            :title="previewPlaying ? t('lcd.configCard.pausePreview') : t('lcd.configCard.playPreview')"
            @click="togglePreviewPlayback"
          >
            <Pause v-if="previewPlaying" :size="12" />
            <Play v-else :size="12" />
          </button>
        </div>
      </aside>
    </div>

    <!-- Custom template -->
    <template v-if="entry.type === 'custom'">
      <div class="template-section">
        <label class="muted">{{ t("lcd.configCard.template") }}</label>
        <!-- Preview thumbnail + dropdown/buttons on the same row (mirrors Slint). -->
        <div class="template-row">
          <div class="template-preview" :style="{ '--preview-brightness': brightness / 100 }">
            <img v-if="previewJpeg" :src="`data:image/jpeg;base64,${previewJpeg}`" :alt="t('lcd.configCard.templatePreviewAlt')" />
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
              <n-button size="small" @click="openEditor()">{{ t("lcd.configCard.newTemplate") }}</n-button>
              <n-button size="small" :disabled="!entry.template_id" @click="entry.template_id && openEditor(entry.template_id)">{{ t("common.edit") }}</n-button>
              <n-button size="small" :disabled="!entry.template_id" @click="duplicateTemplate">{{ t("lcd.configCard.duplicate") }}</n-button>
              <n-button size="small" class="btn-danger" :disabled="!entry.template_id" @click="deleteTemplate">{{ t("common.delete") }}</n-button>
              <n-button size="small" @click="openBrowser">{{ t("lcd.configCard.browseOnline") }}</n-button>
            </div>
          </div>
        </div>
        <div class="checkboxes">
          <n-checkbox :checked="entry.smooth_edges ?? false" @update:checked="(v) => { entry.smooth_edges = v; config.markDirty(); }">{{ t("lcd.configCard.smoothEdges") }}</n-checkbox>
          <n-checkbox v-if="deviceSupportsH264" :checked="entry.custom_h264 ?? true" @update:checked="(v) => { entry.custom_h264 = v; config.markDirty(); }">{{ t("lcd.configCard.h264Streaming") }}</n-checkbox>
        </div>
      </div>
    </template>
    </section>

    <div v-if="supportsCCommand" class="checkboxes">
      <n-checkbox :checked="entry.aio_512_frame ?? aio512Default" @update:checked="(v) => { entry.aio_512_frame = v; config.markDirty(); }">{{ t("lcd.configCard.hidFrame512") }}</n-checkbox>
    </div>

    </div>
  </div>
</template>

<style scoped>
.lcd-config {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-2);
  font-family: var(--font-display);
  font-size: var(--font-size-xl);
  font-weight: 600;
}
.panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: 18px 20px 20px;
  border-radius: 22px;
  background: var(--md-surface-container);
}
.panel-title {
  margin: 0;
  font-size: var(--font-size-sm);
  font-weight: 700;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}
.field > label.muted,
.template-section > label.muted {
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
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
  padding: 3px 10px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--warning) 18%, transparent);
  font-family: var(--font-sans);
  font-size: var(--font-size-xs);
  font-weight: 600;
  color: var(--warning);
}
.cleaner-btn {
  color: var(--warning);
  transition: filter 0.2s ease, transform 0.15s ease;
}
.cleaner-btn:hover:not(:disabled) {
  filter: drop-shadow(0 0 5px color-mix(in srgb, var(--warning) 55%, transparent));
}
.cleaner-btn-running {
  color: var(--warning);
  animation: pulse-glow 2s infinite ease-in-out;
}
@keyframes pulse-glow {
  0%,
  100% {
    filter: drop-shadow(0 0 2px color-mix(in srgb, var(--warning) 40%, transparent));
    opacity: 0.85;
  }
  50% {
    filter: drop-shadow(0 0 8px color-mix(in srgb, var(--warning) 85%, transparent));
    opacity: 1;
  }
}
.card-cleaner-notice {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-radius: 16px;
  background: var(--md-tertiary-container);
  font-size: var(--font-size-sm);
  color: var(--md-on-tertiary-container);
}
.cleaner-notice-icon {
  flex-shrink: 0;
}
.card-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
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
.media-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: var(--space-6);
  align-items: start;
}
.media-layout.with-preview {
  grid-template-columns: minmax(0, 1fr) 240px;
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
  border-radius: 20px;
  box-shadow: 0 0 0 6px var(--md-surface-container-highest), 0 12px 28px -12px rgba(0, 0, 0, 0.6);
}
.preview-frame img,
.preview-frame video,
.preview-color {
  width: 100%;
  height: 100%;
  object-fit: fill;
}
.preview-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  margin-top: var(--space-3);
}
.preview-caption {
  font-size: var(--font-size-xs);
  color: var(--md-on-surface-variant);
}
.preview-play {
  display: inline-flex;
  padding: 6px;
  border: none;
  border-radius: 50%;
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
  cursor: pointer;
}
.preview-play:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
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
    width: 220px;
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
  width: 120px;
  height: 120px;
  flex-shrink: 0;
  background: #000;
  border-radius: 18px;
  box-shadow: 0 0 0 4px var(--md-surface-container-highest);
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
  --n-color-hover: color-mix(in srgb, var(--danger) 14%, transparent) !important;
  --n-border-hover: 1px solid var(--danger) !important;
  --n-text-color-hover: var(--danger) !important;
  --n-color-pressed: color-mix(in srgb, var(--danger) 22%, transparent) !important;
  --n-border-pressed: 1px solid var(--danger) !important;
  --n-text-color-pressed: var(--danger) !important;
}
.checkboxes {
  display: flex;
  gap: var(--space-4);
  flex-wrap: wrap;
}
</style>
