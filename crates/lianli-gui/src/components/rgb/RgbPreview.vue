<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { RgbDeviceCapabilities, RgbDeviceConfig, RgbPreview } from "@/types";
import { useIpc } from "@/composables/useIpc";
import { useRgbStore } from "@/stores/rgb";
import { useTranslate } from "@/i18n";

const props = defineProps<{
  cap: RgbDeviceCapabilities;
  config: RgbDeviceConfig;
  mbSync: boolean;
}>();

const t = useTranslate();
const ipc = useIpc();
const rgb = useRgbStore();

const ELEMENT_HEIGHT = 132;
const GAP = 20;
const LABEL_HEIGHT = 22;
const PAD = 20;

const preview = ref<RgbPreview | null>(null);
const frames = ref<Uint8Array | null>(null);
const error = ref("");
const loading = ref(false);
const root = ref<HTMLElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const width = ref(0);

const caption = computed(() => {
  if (props.mbSync) return t("rgb.preview.mbSync");
  if (error.value) return error.value;
  if (!preview.value) return loading.value ? t("rgb.preview.loading") : t("rgb.preview.unavailable");
  return preview.value.fidelity === "exact" ? t("rgb.preview.caption") : t("rgb.preview.approximate");
});

const boxes = computed(() => {
  const elements = preview.value?.elements ?? [];
  const natural = elements.reduce((sum, element) => sum + element.aspect * ELEMENT_HEIGHT, 0) + GAP * Math.max(0, elements.length - 1);
  const room = width.value - 2 * PAD;
  const scale = natural > 0 && room > 0 ? Math.min(1, room / natural) : 1;
  const height = ELEMENT_HEIGHT * scale;
  let x = 0;
  return elements.map((element) => {
    const box = { x, width: element.aspect * height, height, shape: element.shape };
    x += box.width + GAP * scale;
    return box;
  });
});

const labels = computed(() => {
  let fan = 0;
  return boxes.value.map((box) => {
    if (box.shape === "fan") return t("rgb.preview.fan", { index: ++fan });
    if (box.shape === "pump") return t("rgb.preview.pump");
    return "";
  });
});

let request = 0;
let debounce: ReturnType<typeof setTimeout> | undefined;
async function load() {
  const id = ++request;
  loading.value = true;
  try {
    const result = await ipc.request<RgbPreview | null>("RenderRgbPreview", { device_id: props.cap.device_id, config: props.config });
    if (id !== request) return;
    error.value = "";
    preview.value = result;
    frames.value = result ? Uint8Array.from(atob(result.frames), (c) => c.charCodeAt(0)) : null;
  } catch (e) {
    if (id !== request) return;
    preview.value = null;
    frames.value = null;
    error.value = String(e);
  } finally {
    if (id === request) loading.value = false;
  }
}

watch(
  () => [props.cap.device_id, props.mbSync, rgb.directEdits, JSON.stringify(props.config)],
  () => {
    clearTimeout(debounce);
    if (props.mbSync) return;
    debounce = setTimeout(() => void load(), 200);
  },
  { immediate: true },
);

let shown = -1;
function draw(index: number) {
  const el = canvas.value;
  const data = frames.value;
  const layout = preview.value;
  if (!el || !layout || !data) return;
  const ratio = window.devicePixelRatio || 1;
  const height = (boxes.value[0]?.height ?? ELEMENT_HEIGHT) + LABEL_HEIGHT + 2 * PAD;
  const total = boxes.value.reduce((end, box) => Math.max(end, box.x + box.width), 0) + 2 * PAD;
  if (el.width !== Math.round(total * ratio) || el.height !== Math.round(height * ratio)) {
    el.width = Math.round(total * ratio);
    el.height = Math.round(height * ratio);
    el.style.width = `${total}px`;
    el.style.height = `${height}px`;
  }
  const ctx = el.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, total, height);
  ctx.translate(PAD, PAD);
  for (const box of boxes.value) {
    ctx.fillStyle = "#16161d";
    ctx.beginPath();
    if (box.shape === "pump") ctx.arc(box.x + box.width / 2, box.height / 2, box.height / 2, 0, Math.PI * 2);
    else ctx.roundRect(box.x, 0, box.width, box.height, Math.min(18, box.height / 5));
    ctx.fill();
  }
  const perFrame = layout.leds.length * 3;
  const offset = (index % layout.frame_count) * perFrame;
  const size = Math.max(2.5, Math.min(6, (boxes.value[0]?.height ?? ELEMENT_HEIGHT) / 26));
  layout.leds.forEach((led, i) => {
    const box = boxes.value[led.element];
    if (!box) return;
    const [r, g, b] = [data[offset + i * 3], data[offset + i * 3 + 1], data[offset + i * 3 + 2]];
    const x = box.x + led.x * box.width;
    const y = led.y * box.height;
    const lit = r + g + b > 24;
    ctx.shadowBlur = lit ? size * 2.5 : 0;
    ctx.shadowColor = `rgb(${r}, ${g}, ${b})`;
    ctx.fillStyle = lit ? `rgb(${r}, ${g}, ${b})` : "#2a2a32";
    ctx.beginPath();
    ctx.arc(x, y, size, 0, Math.PI * 2);
    ctx.fill();
  });
  ctx.shadowBlur = 0;
  ctx.fillStyle = "rgba(232, 232, 238, 0.75)";
  ctx.font = "600 12px system-ui, sans-serif";
  ctx.textAlign = "center";
  boxes.value.forEach((box, i) => {
    if (labels.value[i]) ctx.fillText(labels.value[i], box.x + box.width / 2, box.height + 16);
  });
}

let raf = 0;
let started = 0;
let visible = true;
function tick(now: number) {
  raf = 0;
  const layout = preview.value;
  if (!layout || !visible || document.hidden) return;
  if (!started) started = now;
  const index = layout.frame_count > 1 ? Math.floor((now - started) / Math.max(1, layout.interval_ms)) % layout.frame_count : 0;
  if (index !== shown) {
    shown = index;
    draw(index);
  }
  if (layout.frame_count > 1) raf = requestAnimationFrame(tick);
}

function restart() {
  if (raf) cancelAnimationFrame(raf);
  shown = -1;
  started = 0;
  raf = requestAnimationFrame(tick);
}

watch([preview, boxes], restart);

let observer: IntersectionObserver | undefined;
let resize: ResizeObserver | undefined;
function onVisibility() {
  if (!document.hidden) restart();
}
onMounted(() => {
  if (!root.value) return;
  observer = new IntersectionObserver(([entry]) => {
    visible = entry?.isIntersecting ?? true;
    if (visible) restart();
  });
  observer.observe(root.value);
  resize = new ResizeObserver(([entry]) => {
    width.value = Math.max(0, (entry?.contentRect.width ?? 0));
  });
  resize.observe(root.value);
  document.addEventListener("visibilitychange", onVisibility);
});
onUnmounted(() => {
  request += 1;
  clearTimeout(debounce);
  if (raf) cancelAnimationFrame(raf);
  observer?.disconnect();
  resize?.disconnect();
  document.removeEventListener("visibilitychange", onVisibility);
});
</script>

<template>
  <div class="preview">
    <span class="caption" role="status">{{ caption }}</span>
    <div ref="root" class="stage" role="img" :aria-label="t('rgb.preview.label')">
      <canvas v-show="preview && !mbSync" ref="canvas" />
    </div>
  </div>
</template>

<style scoped>
.preview {
  position: relative;
  padding: 28px var(--space-2) var(--space-1);
  border-radius: 24px;
  background: #0b0b10;
  color: #e8e8ee;
}
.caption {
  position: absolute;
  top: 14px;
  left: 16px;
  max-width: calc(100% - 32px);
  padding: 4px 12px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.09);
  font-size: var(--font-size-xs);
  font-weight: 600;
}
.stage {
  display: flex;
  justify-content: center;
  min-height: 24px;
}
</style>
