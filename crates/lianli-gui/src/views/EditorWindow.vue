<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { Copy, Save, X } from "lucide-vue-next";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { LcdTemplate, Widget, Widget as W } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useLcdStore } from "@/stores/lcd";
import { useFonts } from "@/composables/useFonts";
import WidgetList from "@/components/editor/WidgetList.vue";
import WidgetCanvas from "@/components/editor/WidgetCanvas.vue";
import PropertiesPanel from "@/components/editor/PropertiesPanel.vue";
import { screenPresets } from "@/constants/screen";

const config = useConfigStore();
const lcd = useLcdStore();
const route = useRoute();
const { load: loadFonts, fontOptions } = useFonts();

const template = ref<LcdTemplate | null>(null);
const selectedId = ref<string | null>(null);
const statusMessage = ref("");
const statusIsError = ref(false);

const renderPreview = lcd.renderPreview();

const sensorOptions = computed(() =>
  config.sensors.map((s) => ({
    label: s.display_name ?? s.sensor_name?.sensor_name ?? "sensor",
    value: JSON.stringify(s.source),
  })),
);

const presetOptions = screenPresets.map((p) => ({ label: p.label, value: `${p.width}x${p.height}` }));

/** Load the template identified by `?template=<id>` (falls back to first). */
function loadRequestedTemplate(id: string | (string | null)[] | null) {
  const tid = Array.isArray(id) ? id[0] : id;
  const match = tid ? config.templates.find((t) => t.id === tid) : undefined;
  template.value = match
    ? (JSON.parse(JSON.stringify(match)) as LcdTemplate)
    : config.templates.length
      ? (JSON.parse(JSON.stringify(config.templates[0])) as LcdTemplate)
      : blankTemplate();
  selectedId.value = null;
  triggerPreview();
}

onMounted(async () => {
  await Promise.all([config.load().catch(() => undefined), loadFonts()]);
  loadRequestedTemplate(route.query.template);
  window.addEventListener("keydown", onKeyDown);
});

// Re-load when the query changes (e.g. the user clicks "Edit" on a different
// template while the editor window is already open).
watch(
  () => route.query.template,
  (id) => {
    if (template.value && id && id !== template.value.id) loadRequestedTemplate(id);
  },
);

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
});

function blankTemplate(): LcdTemplate {
  return {
    id: "user-" + Date.now().toString(16),
    name: "New Template",
    base_width: 480,
    base_height: 480,
    background: { type: "color", rgb: [0, 0, 0, 255] },
    widgets: [],
    rotated: false,
    target_device: null,
  };
}

/**
 * Rotate the template 90° in place (mirrors the Slint editor's
 * `ops::apply_rotation_swap`). Toggling "Rotated" swaps base_width/height and
 * re-maps every widget's centre/size so the authored layout is re-oriented,
 * rather than just flipping a display flag. The preview then renders at the
 * (now swapped) base dimensions.
 *
 * `toRotated` true → 90° CW (e.g. 480×1920 → 1920×480); false → inverse.
 */
function applyRotationSwap(tpl: LcdTemplate, toRotated: boolean) {
  const oldW = tpl.base_width;
  const oldH = tpl.base_height;
  tpl.base_width = oldH;
  tpl.base_height = oldW;
  for (const w of tpl.widgets) {
    const cx = w.x;
    const cy = w.y;
    const ww = w.width;
    const wh = w.height;
    if (toRotated) {
      w.x = oldH - cy;
      w.y = cx;
    } else {
      w.x = cy;
      w.y = oldW - cx;
    }
    w.width = wh;
    w.height = ww;
  }
}

function onRotated(v: boolean) {
  const tpl = template.value;
  if (!tpl) return;
  if (v !== tpl.rotated && tpl.base_width !== tpl.base_height) {
    applyRotationSwap(tpl, v);
  }
  tpl.rotated = v;
  triggerPreview();
}

const selectedWidget = computed<Widget | null>(
  () => template.value?.widgets.find((w) => w.id === selectedId.value) ?? null,
);

function onSelect(id: string | null) {
  selectedId.value = id;
}

function onAdd(kindId: string) {
  if (!template.value) return;
  const id = `${kindId}-${template.value.widgets.length + 1}`;
  const w: Widget = {
    id,
    kind: defaultKind(kindId),
    x: template.value.base_width / 2,
    y: template.value.base_height / 2,
    width: 120,
    height: 60,
    rotation: 0,
    visible: true,
  };
  template.value.widgets.push(w);
  selectedId.value = id;
  triggerPreview();
}

function defaultKind(kindId: string): any {
  const white: [number, number, number, number] = [255, 255, 255, 255];
  switch (kindId) {
    case "label":
      return { type: "label", text: "Label", font: { path: null }, font_size: 28, color: white, align: "center", letter_spacing: 0 };
    case "value_text":
      return { type: "value_text", source: { type: "cpu_usage" }, format: "{:.0}", unit: "%", font: { path: null }, font_size: 48, color: white, align: "center", value_min: 0, value_max: 100, ranges: [], letter_spacing: 0 };
    case "radial_gauge":
      return { type: "radial_gauge", source: { type: "cpu_usage" }, value_min: 0, value_max: 100, start_angle: 90, sweep_angle: 330, inner_radius_pct: 0.78, background_color: [30, 30, 30, 255], ranges: [], bg_corner_radius: 0, value_corner_radius: 0, gradient: false, gradient_stops: [] };
    case "vertical_bar":
      return { type: "vertical_bar", source: { type: "cpu_usage" }, value_min: 0, value_max: 100, background_color: [30, 30, 30, 255], corner_radius: 0, ranges: [] };
    case "horizontal_bar":
      return { type: "horizontal_bar", source: { type: "cpu_usage" }, value_min: 0, value_max: 100, background_color: [30, 30, 30, 255], corner_radius: 0, ranges: [] };
    case "speedometer":
      return { type: "speedometer", source: { type: "cpu_usage" }, value_min: 0, value_max: 100, start_angle: 135, sweep_angle: 270, needle_color: [255, 255, 255, 255], tick_color: [200, 200, 200, 255], tick_count: 10, background_color: [30, 30, 30, 255], ranges: [], show_gauge: true, show_needle: true, needle_width: 14, needle_length_pct: 0.95, needle_border_color: [174, 10, 16, 255], needle_border_width: 1.5 };
    case "core_bars":
      return { type: "core_bars", orientation: "horizontal", background_color: [30, 30, 30, 255], show_labels: true, ranges: [] };
    case "image":
      return { type: "image", path: "", opacity: 1, fit: "stretch" };
    case "video":
      return { type: "video", path: "", loop_playback: true, opacity: 1, fit: "stretch" };
    case "clock_digital":
      return { type: "clock_digital", format: "%H:%M", font: { path: null }, font_size: 64, color: white, align: "center", letter_spacing: 0 };
    case "clock_analog":
      return { type: "clock_analog", face_color: [30, 30, 30, 255], tick_color: [220, 220, 220, 255], minor_tick_color: [220, 220, 220, 255], hour_hand_color: [240, 240, 240, 255], minute_hand_color: [240, 240, 240, 255], second_hand_color: [220, 40, 40, 255], hub_color: [240, 240, 240, 255], numbers_color: [230, 230, 230, 255], numbers_font: { path: null }, numbers_font_size: 24, show_seconds: true, show_hour_ticks: true, show_minor_ticks: true, show_numbers: false, hour_hand_width: 6, minute_hand_width: 4, second_hand_width: 2, hour_hand_length_pct: 0.55, minute_hand_length_pct: 0.8, second_hand_length_pct: 0.9, hour_tick_length_pct: 0.12, minor_tick_length_pct: 0.05, hour_tick_width: 3, minor_tick_width: 1.5, hub_radius: 6 };
    case "sparkline":
      return { type: "sparkline", source: { type: "cpu_usage" }, value_min: 0, value_max: 100, auto_range: false, history_length: 60, line_width: 2, line_color: [80, 180, 240, 255], fill_color: [80, 180, 240, 80], background_color: [0, 0, 0, 255], ranges: [], border_color: [80, 90, 110, 255], border_width: 0, corner_radius: 0, padding: 0, show_points: false, point_radius: 2.5, show_baseline: false, baseline_value: 0, baseline_color: [140, 140, 160, 160], baseline_width: 1, smooth: false, scroll_rtl: false, fill_from_ranges: false, range_blend: false, show_gridlines: false, gridlines_horizontal: 3, gridlines_vertical: 0, gridline_color: [120, 120, 140, 90], gridline_width: 1, show_axis_labels: false, axis_label_count: 3, axis_labels_on_right: false, axis_label_format: "{:.0}", axis_label_font: { path: null }, axis_label_size: 11, axis_label_color: [200, 200, 210, 220], axis_label_padding: 4 };
    default:
      return { type: kindId };
  }
}

function onRemove(id: string) {
  if (!template.value) return;
  template.value.widgets = template.value.widgets.filter((w) => w.id !== id);
  if (selectedId.value === id) selectedId.value = null;
  triggerPreview();
}

function onMove(id: string, dir: -1 | 1) {
  if (!template.value) return;
  const arr = template.value.widgets;
  const i = arr.findIndex((w) => w.id === id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= arr.length) return;
  [arr[i], arr[j]] = [arr[j], arr[i]];
}

// ── Duplicate / clipboard ───────────────────────────────────────────────────
function nextWidgetId(kindId: string): string {
  const existing = template.value?.widgets ?? [];
  let n = existing.length + 1;
  let id = `${kindId}-${n}`;
  while (existing.some((w) => w.id === id)) {
    n++;
    id = `${kindId}-${n}`;
  }
  return id;
}

function onDuplicate(id: string) {
  const src = template.value?.widgets.find((w) => w.id === id);
  if (!src || !template.value) return;
  const copy: Widget = JSON.parse(JSON.stringify(src));
  copy.id = nextWidgetId(src.kind.type ?? "widget");
  copy.x += 20;
  copy.y += 20;
  const idx = template.value.widgets.indexOf(src);
  template.value.widgets.splice(idx + 1, 0, copy);
  selectedId.value = copy.id;
  triggerPreview();
}

// Cross-widget clipboard (Ctrl+C / Ctrl+V). Stored as a plain JSON snapshot.
const clipboard = ref<Widget | null>(null);

function copySelected() {
  if (!selectedId.value || !template.value) return;
  const sel = template.value.widgets.find((w) => w.id === selectedId.value);
  if (sel) clipboard.value = JSON.parse(JSON.stringify(sel));
}

function pasteClipboard() {
  if (!clipboard.value || !template.value) return;
  const copy: Widget = JSON.parse(JSON.stringify(clipboard.value));
  copy.id = nextWidgetId(copy.kind?.type ?? "widget");
  copy.x += 24;
  copy.y += 24;
  template.value.widgets.push(copy);
  selectedId.value = copy.id;
  triggerPreview();
}

// ── Keyboard shortcuts ──────────────────────────────────────────────────────
function onKeyDown(e: KeyboardEvent) {
  // Don't hijack typing in form fields.
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) {
    return;
  }
  const mod = e.ctrlKey || e.metaKey;

  // Copy / paste / duplicate work without a selection guard for copy (no-op),
  // but paste/duplicate need a source.
  if (mod && (e.key === "c" || e.key === "C")) {
    e.preventDefault();
    copySelected();
    return;
  }
  if (mod && (e.key === "v" || e.key === "V")) {
    e.preventDefault();
    pasteClipboard();
    return;
  }
  if (mod && (e.key === "d" || e.key === "D")) {
    e.preventDefault();
    if (selectedId.value) onDuplicate(selectedId.value);
    return;
  }

  if (!selectedId.value || !template.value) return;
  const sel = template.value.widgets.find((w) => w.id === selectedId.value);
  if (!sel) return;

  const step = e.shiftKey ? 10 : 1;
  let handled = true;
  switch (e.key) {
    case "ArrowLeft":
      sel.x -= step;
      break;
    case "ArrowRight":
      sel.x += step;
      break;
    case "ArrowUp":
      sel.y -= step;
      break;
    case "ArrowDown":
      sel.y += step;
      break;
    case "Delete":
    case "Backspace":
      onRemove(sel.id);
      break;
    default:
      handled = false;
  }
  if (handled) {
    e.preventDefault();
    triggerPreview();
  }
}

function onWidgetMove(id: string, x: number, y: number) {
  const w = template.value?.widgets.find((ww) => ww.id === id);
  if (!w) return;
  w.x = x;
  w.y = y;
  triggerPreview();
}

function patchCommon(id: string, key: string, value: any) {
  const w = template.value?.widgets.find((ww) => ww.id === id);
  if (!w) return;
  (w as any)[key] = value;
  triggerPreview();
}
function patchKind(id: string, key: string, value: any) {
  const w = template.value?.widgets.find((ww) => ww.id === id);
  if (!w) return;
  (w.kind as any)[key] = value;
  triggerPreview();
}

// Debounced preview rendering (200ms).
function triggerPreview() {
  if (!template.value) return;
  renderPreview(template.value, template.value.base_width, template.value.base_height);
}

watch(template, () => triggerPreview(), { deep: true });

async function onCopyJson() {
  if (!template.value) return;
  const json = JSON.stringify(template.value, null, 2);
  try {
    await writeText(json);
    setStatus("Template JSON copied to clipboard", false);
  } catch (e) {
    setStatus(`Copy failed: ${e}`, true);
  }
}

async function onSave() {
  if (!template.value) return;
  const idx = config.templates.findIndex((t) => t.id === template.value!.id);
  if (idx >= 0) {
    config.templates[idx] = JSON.parse(JSON.stringify(template.value));
  } else {
    config.templates.push(JSON.parse(JSON.stringify(template.value)));
  }
  try {
    await lcd.setTemplates(config.templates);
    setStatus("Saved", false);
  } catch (e) {
    setStatus(`Save failed: ${e}`, true);
  }
}

function setStatus(msg: string, isError: boolean) {
  statusMessage.value = msg;
  statusIsError.value = isError;
}

async function closeWindow() {
  try {
    await getCurrentWebviewWindow().close();
  } catch (e) {
    // eslint-disable-next-line no-console
    console.warn("close window failed", e);
  }
}

const localName = ref("");
watch(template, (t) => { localName.value = t?.name ?? ""; }, { immediate: true });
function commitName() {
  if (template.value) {
    template.value.name = localName.value;
  }
}
</script>

<template>
  <div class="editor-window">
    <div class="toolbar">
      <n-input v-model:value="localName" @blur="commitName" size="small" style="width: 220px" placeholder="Template name" />
      <n-select
        :value="`${template?.base_width ?? 480}x${template?.base_height ?? 480}`"
        :options="presetOptions"
        size="small"
        style="width: 200px"
        @update:value="(v: string) => { const [w, h] = v.split('x').map(Number); if (template) { template.base_width = w; template.base_height = h; } }"
      />
      <n-checkbox :checked="template?.rotated ?? false" @update:checked="onRotated">Rotated</n-checkbox>
      <div class="spacer" />
      <span v-if="statusMessage" class="status" :class="{ err: statusIsError }">{{ statusMessage }}</span>
      <n-button size="small" @click="onCopyJson"><template #icon><Copy :size="14" /></template>Copy JSON</n-button>
      <n-button size="small" type="primary" @click="onSave"><template #icon><Save :size="14" /></template>Save</n-button>
      <n-button size="small" quaternary @click="closeWindow"><template #icon><X :size="14" /></template>Close</n-button>
    </div>

    <div class="panes">
      <div class="pane left">
        <WidgetList
          :widgets="template?.widgets ?? []"
          :selected-id="selectedId"
          @select="onSelect"
          @add="onAdd"
          @remove="onRemove"
          @duplicate="onDuplicate"
          @move="onMove"
        />
      </div>
      <div class="pane center">
        <WidgetCanvas
          v-if="template"
          :template="template"
          :selected-id="selectedId"
          :preview-jpeg="lcd.previewJpeg"
          :preview-loading="lcd.previewLoading"
          @select="onSelect"
          @move="onWidgetMove"
        />
      </div>
      <div class="pane right">
        <PropertiesPanel
          :widget="selectedWidget"
          :sensor-options="sensorOptions"
          :font-options="fontOptions()"
          @patch-common="patchCommon"
          @patch-kind="patchKind"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  width: 100vw;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
  flex-shrink: 0;
}
.spacer {
  flex: 1;
}
.status {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
.status.err {
  color: var(--danger);
}
.panes {
  display: flex;
  flex: 1;
  min-height: 0;
}
.pane {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.left {
  width: 240px;
  border-right: 1px solid var(--border);
  padding: var(--space-2);
  flex-shrink: 0;
  overflow: hidden;
}
.center {
  flex: 1;
  min-width: 0;
  padding: var(--space-4);
  background: var(--bg-base);
  display: flex;
  flex-direction: column;
  align-items: stretch;
  overflow: hidden;
}
.right {
  width: 320px;
  border-left: 1px solid var(--border);
  padding: var(--space-3);
  flex-shrink: 0;
  overflow: hidden;
}
</style>
