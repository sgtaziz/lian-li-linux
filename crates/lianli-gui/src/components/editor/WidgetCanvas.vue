<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { LcdTemplate, Widget } from "@/types";

const props = defineProps<{
  template: LcdTemplate;
  selectedId: string | null;
  previewJpeg: string;
  previewLoading: boolean;
}>();

const emit = defineEmits<{
  select: [id: string | null];
  move: [id: string, x: number, y: number];
}>();

// The template's base dimensions are authoritative. Toggling "Rotated" in the
// editor physically swaps these dims and rotates widget coords in-place (see
// EditorWindow.applyRotationSwap), so the canvas always renders base × base.
const effectiveW = computed(() => props.template.base_width);
const effectiveH = computed(() => props.template.base_height);

// The canvas stage fills the available space; we compute the largest
// aspect-correct rectangle that fits inside it so the preview image and the
// draggable widget boxes stay aligned regardless of window size.
const stageEl = ref<HTMLElement | null>(null);
const boxW = ref(0);
const boxH = ref(0);
let ro: ResizeObserver | null = null;

function recompute() {
  const el = stageEl.value;
  if (!el) return;
  const cw = el.clientWidth;
  const ch = el.clientHeight;
  const ew = effectiveW.value;
  const eh = effectiveH.value;
  if (ew <= 0 || eh <= 0 || cw <= 0 || ch <= 0) {
    boxW.value = 0;
    boxH.value = 0;
    return;
  }
  const ratio = ew / eh;
  let w = cw;
  let h = w / ratio;
  if (h > ch) {
    h = ch;
    w = h * ratio;
  }
  boxW.value = Math.floor(w);
  boxH.value = Math.floor(h);
}

onMounted(() => {
  recompute();
  ro = new ResizeObserver(recompute);
  if (stageEl.value) ro.observe(stageEl.value);
});
onUnmounted(() => ro?.disconnect());
watch([effectiveW, effectiveH], recompute);

const canvasStyle = computed(() => ({
  width: boxW.value ? `${boxW.value}px` : "0",
  height: boxH.value ? `${boxH.value}px` : "0",
}));

const previewBg = computed(() => {
  if (props.previewJpeg) {
    return `url(data:image/jpeg;base64,${props.previewJpeg}) center / cover no-repeat`;
  }
  const bg = props.template.background;
  if (bg?.type === "color") {
    const [r, g, b] = bg.rgb;
    return `rgb(${r}, ${g}, ${b})`;
  }
  return "#1a1f29";
});

function widgetBoxStyle(w: Widget) {
  // Map template-space coords into percentages of the fitted canvas.
  const ew = effectiveW.value || 1;
  const eh = effectiveH.value || 1;
  const leftPct = (w.x / ew) * 100;
  const topPct = (w.y / eh) * 100;
  const widthPct = (w.width / ew) * 100;
  const heightPct = (w.height / eh) * 100;
  return {
    left: `${leftPct - widthPct / 2}%`,
    top: `${topPct - heightPct / 2}%`,
    width: `${widthPct}%`,
    height: `${heightPct}%`,
    transform: w.rotation ? `rotate(${w.rotation}deg)` : undefined,
    opacity: w.visible === false ? 0.35 : 1,
  };
}

const dragging = ref<{ id: string; offX: number; offY: number } | null>(null);

function onStageClick() {
  emit("select", null);
}

function onWidgetDown(evt: MouseEvent, w: Widget) {
  evt.stopPropagation();
  emit("select", w.id);
  const stage = stageEl.value!;
  const rect = stage.getBoundingClientRect();
  // Offset of the pointer within the fitted canvas, in template units.
  const px = (w.x / effectiveW.value) * boxW.value;
  const py = (w.y / effectiveH.value) * boxH.value;
  dragging.value = {
    id: w.id,
    offX: evt.clientX - (rect.left + (rect.width - boxW.value) / 2) - px,
    offY: evt.clientY - (rect.top + (rect.height - boxH.value) / 2) - py,
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

function onMove(evt: MouseEvent) {
  if (!dragging.value || !stageEl.value) return;
  const rect = stageEl.value.getBoundingClientRect();
  const ox = (rect.width - boxW.value) / 2;
  const oy = (rect.height - boxH.value) / 2;
  const localX = evt.clientX - rect.left - ox - dragging.value.offX;
  const localY = evt.clientY - rect.top - oy - dragging.value.offY;
  const tx = (localX / boxW.value) * effectiveW.value;
  const ty = (localY / boxH.value) * effectiveH.value;
  emit("move", dragging.value.id, Math.round(tx), Math.round(ty));
}

function onUp() {
  dragging.value = null;
  window.removeEventListener("mousemove", onMove);
  window.removeEventListener("mouseup", onUp);
}
</script>

<template>
  <div class="canvas-wrap">
    <div ref="stageEl" class="stage" @click="onStageClick">
      <div class="canvas" :style="[canvasStyle, { background: previewBg }]">
        <div
          v-for="w in template.widgets"
          :key="w.id"
          class="widget-box"
          :class="{ selected: w.id === selectedId }"
          :style="widgetBoxStyle(w)"
          @mousedown="onWidgetDown($event, w)"
          @click.stop="emit('select', w.id)"
        >
          <span class="widget-tag">{{ w.id }}</span>
        </div>
      </div>
    </div>
    <div v-if="previewLoading" class="loading">Rendering preview…</div>
    <div class="dims mono">{{ effectiveW }} × {{ effectiveH }}</div>
  </div>
</template>

<style scoped>
.canvas-wrap {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  flex: 1;
  min-height: 0;
  width: 100%;
  position: relative;
}
.stage {
  flex: 1;
  width: 100%;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  /* dark checkerboard so a transparent/letterboxed canvas is visible */
  background:
    repeating-conic-gradient(#161922 0% 25%, #11141c 0% 50%) 50% / 24px 24px;
  border-radius: var(--radius-md);
  overflow: hidden;
  cursor: default;
}
.canvas {
  position: relative;
  background-size: cover;
  background-position: center;
  border: 1px solid var(--border-strong);
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.4);
}
.widget-box {
  position: absolute;
  border: 1px dashed rgba(79, 158, 255, 0.55);
  border-radius: 2px;
  cursor: grab;
}
.widget-box.selected {
  border: 1px solid var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}
.widget-tag {
  position: absolute;
  top: 2px;
  left: 2px;
  font-size: 9px;
  color: var(--accent);
  background: rgba(0, 0, 0, 0.55);
  padding: 1px 3px;
  border-radius: 2px;
  pointer-events: none;
}
.loading {
  position: absolute;
  top: var(--space-3);
  left: 50%;
  transform: translateX(-50%);
  padding: 3px var(--space-3);
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: 999px;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  pointer-events: none;
  z-index: 2;
}
.dims {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
</style>
