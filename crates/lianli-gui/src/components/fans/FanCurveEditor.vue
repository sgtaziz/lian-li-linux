<script setup lang="ts">
import { computed, ref } from "vue";

const props = defineProps<{
  modelValue: [number, number][];
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: [number, number][]];
}>();

// X axis: 20–100 °C. Y axis: 0–100 %.
const X_MIN = 20;
const X_MAX = 100;
const Y_MIN = 0;
const Y_MAX = 100;
const W = 480;
const H = 300;
const PAD = 34;

const hoverIndex = ref<number | null>(null);
const dragIndex = ref<number | null>(null);

const innerW = computed(() => W - PAD * 2);
const innerH = computed(() => H - PAD * 2);

function xToPx(x: number): number {
  return PAD + ((x - X_MIN) / (X_MAX - X_MIN)) * innerW.value;
}
function yToPx(y: number): number {
  return PAD + (1 - (y - Y_MIN) / (Y_MAX - Y_MIN)) * innerH.value;
}
function pxToX(px: number): number {
  return X_MIN + ((px - PAD) / innerW.value) * (X_MAX - X_MIN);
}
function pxToY(py: number): number {
  return Y_MIN + (1 - (py - PAD) / innerH.value) * (Y_MAX - Y_MIN);
}

const sorted = computed(() => {
  const copy = [...props.modelValue].map((p, i) => ({ p, i }));
  copy.sort((a, b) => a.p[0] - b.p[0]);
  return copy;
});

const linePath = computed(() => {
  const pts = sorted.value;
  if (pts.length === 0) return "";
  return pts
    .map((pt, idx) => `${idx === 0 ? "M" : "L"} ${xToPx(pt.p[0])} ${yToPx(pt.p[1])}`)
    .join(" ");
});

const xGrid = computed(() => {
  const arr: number[] = [];
  for (let x = X_MIN; x <= X_MAX; x += 10) arr.push(x);
  return arr;
});
const yGrid = computed(() => {
  const arr: number[] = [];
  for (let y = Y_MIN; y <= Y_MAX; y += 20) arr.push(y);
  return arr;
});

function svgPoint(evt: MouseEvent | DragEvent): { x: number; y: number } {
  const svg = (evt.currentTarget as SVGElement).closest("svg")!;
  const rect = svg.getBoundingClientRect();
  const scaleX = W / rect.width;
  const scaleY = H / rect.height;
  return {
    x: (evt.clientX - rect.left) * scaleX,
    y: (evt.clientY - rect.top) * scaleY,
  };
}

function onCanvasClick(evt: MouseEvent) {
  if (props.disabled || dragIndex.value !== null) return;
  const { x, y } = svgPoint(evt);
  if (x < PAD || x > W - PAD || y < PAD || y > H - PAD) return;
  const temp = clamp(pxToX(x), X_MIN, X_MAX);
  const speed = clamp(pxToY(y), Y_MIN, Y_MAX);
  const next = [...props.modelValue, [round1(temp), round0(speed)] as [number, number]];
  next.sort((a, b) => a[0] - b[0]);
  emit("update:modelValue", next);
}

function onPointMouseDown(evt: MouseEvent, index: number) {
  if (props.disabled) return;
  evt.stopPropagation();
  dragIndex.value = index;
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(evt: MouseEvent) {
  if (dragIndex.value === null) return;
  const svg = document.getElementById("curve-svg");
  if (!svg) return;
  const rect = svg.getBoundingClientRect();
  const scaleX = W / rect.width;
  const scaleY = H / rect.height;
  const x = (evt.clientX - rect.left) * scaleX;
  const y = (evt.clientY - rect.top) * scaleY;
  const temp = clamp(pxToX(x), X_MIN, X_MAX);
  const speed = clamp(pxToY(y), Y_MIN, Y_MAX);
  const next = props.modelValue.map((p, i) =>
    i === dragIndex.value ? ([round1(temp), round0(speed)] as [number, number]) : p,
  );
  next.sort((a, b) => a[0] - b[0]);
  emit("update:modelValue", next);
}

function onWindowMouseUp() {
  dragIndex.value = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

function onPointDblClick(evt: MouseEvent, index: number) {
  if (props.disabled) return;
  evt.stopPropagation();
  if (props.modelValue.length <= 1) return; // keep at least one point
  const next = props.modelValue.filter((_, i) => i !== index);
  emit("update:modelValue", next);
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}
function round1(v: number): number {
  return Math.round(v * 10) / 10;
}
function round0(v: number): number {
  return Math.round(v);
}

function pointPx(pt: [number, number]) {
  return { cx: xToPx(pt[0]), cy: yToPx(pt[1]) };
}
</script>

<template>
  <div class="curve-editor">
    <svg
      id="curve-svg"
      :viewBox="`0 0 ${W} ${H}`"
      class="svg"
      :class="{ disabled }"
      @click="onCanvasClick"
    >
      <!-- gridlines -->
      <g class="grid">
        <line
          v-for="x in xGrid"
          :key="'x' + x"
          :x1="xToPx(x)"
          :x2="xToPx(x)"
          :y1="PAD"
          :y2="H - PAD"
        />
        <line
          v-for="y in yGrid"
          :key="'y' + y"
          :y1="yToPx(y)"
          :y2="yToPx(y)"
          :x1="PAD"
          :x2="W - PAD"
        />
      </g>
      <!-- axis labels -->
      <g class="axis-label">
        <text v-for="x in xGrid" :key="'xl' + x" :x="xToPx(x)" :y="H - PAD + 14" text-anchor="middle">
          {{ x }}
        </text>
        <text v-for="y in yGrid" :key="'yl' + y" :x="PAD - 6" :y="yToPx(y) + 3" text-anchor="end">
          {{ y }}
        </text>
        <text :x="W / 2" :y="H - 4" text-anchor="middle" class="axis-title">Temperature (°C)</text>
        <text
          :x="12"
          :y="H / 2"
          text-anchor="middle"
          class="axis-title"
          :transform="`rotate(-90 12 ${H / 2})`"
        >
          Speed (%)
        </text>
      </g>
      <!-- curve -->
      <path :d="linePath" class="curve" v-if="linePath" />
      <!-- points -->
      <g v-for="(pt, idx) in sorted" :key="pt.i">
        <circle
          :cx="pointPx(pt.p).cx"
          :cy="pointPx(pt.p).cy"
          r="7"
          class="point-hit"
          @mousedown="onPointMouseDown($event, pt.i)"
          @click.stop
          @dblclick="onPointDblClick($event, pt.i)"
          @mouseenter="hoverIndex = pt.i"
          @mouseleave="hoverIndex = null"
        />
        <circle
          :cx="pointPx(pt.p).cx"
          :cy="pointPx(pt.p).cy"
          r="5"
          class="point"
          :class="{ hover: hoverIndex === pt.i }"
        />
        <g v-if="hoverIndex === pt.i" class="tooltip">
          <rect
            :x="pointPx(pt.p).cx - 52"
            :y="pointPx(pt.p).cy - 34"
            width="104"
            height="22"
            rx="4"
          />
          <text :x="pointPx(pt.p).cx" :y="pointPx(pt.p).cy - 19" text-anchor="middle">
            {{ pt.p[0] }}°C → {{ pt.p[1] }}%
          </text>
        </g>
      </g>
    </svg>
    <div class="hint">Click to add points. Drag to move. Double-click to remove.</div>
  </div>
</template>

<style scoped>
.curve-editor {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.svg {
  display: block;
  /* Fixed height keeps the plot compact instead of stretching to fill the
     full card width (which made it ~650px tall). Width follows the viewBox. */
  height: 360px;
  width: auto;
  max-width: 100%;
  margin: 0 auto;
  background: #14171f;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  user-select: none;
}
.svg.disabled {
  opacity: 0.5;
  pointer-events: none;
}
.grid line {
  stroke: var(--border);
  stroke-width: 1;
}
.axis-label text {
  fill: var(--text-muted);
  font-size: 10px;
}
.axis-title {
  fill: var(--text-secondary);
  font-size: 11px;
}
.curve {
  fill: none;
  stroke: var(--accent);
  stroke-width: 2;
}
.point {
  fill: var(--accent);
  stroke: #fff;
  stroke-width: 1.5;
  pointer-events: none;
  transition: r 0.1s;
}
.point.hover {
  r: 6.5;
}
.point-hit {
  fill: transparent;
  cursor: grab;
}
.point-hit:active {
  cursor: grabbing;
}
.tooltip rect {
  fill: var(--bg-elevated);
  stroke: var(--border-strong);
}
.tooltip text {
  fill: var(--text-primary);
  font-size: 11px;
}
.hint {
  color: var(--text-muted);
  font-size: var(--font-size-xs);
}
</style>
