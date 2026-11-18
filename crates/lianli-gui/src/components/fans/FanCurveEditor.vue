<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { useTranslate } from "@/i18n";
import {
  CURVE_TEMP_MAX,
  CURVE_TEMP_MIN,
  MAX_CURVE_POINTS,
  canInsertCurvePoint,
  curveAxisRange,
  speedAt,
  type CurveAxisRange,
} from "./curvePoints";
import { newCurveGesture } from "./useCurveHistory";

const props = defineProps<{
  modelValue: [number, number][];
  disabled?: boolean;
  liveCelsius?: number | null;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: [number, number][], gesture: number | null];
}>();

const t = useTranslate();

// Y axis: 0-100 %. The X axis comes from the points (see curveAxisRange).
const Y_MIN = 0;
const Y_MAX = 100;
const W = 480;
const H = 300;
const PAD = 34;

const svgRef = ref<SVGSVGElement | null>(null);
const hoverIndex = ref<number | null>(null);
const dragIndex = ref<number | null>(null);
let dragGesture: number | null = null;
// Held while dragging, so the axis doesn't rescale under the cursor.
const dragRange = ref<CurveAxisRange | null>(null);

const xRange = computed(() => dragRange.value ?? curveAxisRange(props.modelValue));
const X_MIN = computed(() => xRange.value.min);
const X_MAX = computed(() => xRange.value.max);

const innerW = computed(() => W - PAD * 2);
const innerH = computed(() => H - PAD * 2);

function xToPx(x: number): number {
  return PAD + ((x - X_MIN.value) / (X_MAX.value - X_MIN.value)) * innerW.value;
}
function yToPx(y: number): number {
  return PAD + (1 - (y - Y_MIN) / (Y_MAX - Y_MIN)) * innerH.value;
}
function pxToX(px: number): number {
  return X_MIN.value + ((px - PAD) / innerW.value) * (X_MAX.value - X_MIN.value);
}
function pxToY(py: number): number {
  return Y_MIN + (1 - (py - PAD) / innerH.value) * (Y_MAX - Y_MIN);
}

const sorted = computed(() => {
  const copy = [...props.modelValue].map((p, i) => ({ p, i }));
  copy.sort((a, b) => a.p[0] - b.p[0]);
  return copy;
});

// The axis stops at 0-120 °C, but a hand-edited config can hold points past it, so the line is cut there.
const visibleLine = computed<[number, number][]>(() => {
  const pts = sorted.value.map((pt) => pt.p);
  if (pts.length === 0) return [];
  const [lo, hi] = [X_MIN.value, X_MAX.value];
  const line: [number, number][] = [];
  if (pts[0][0] < lo) line.push([lo, speedAt(pts, lo)]);
  line.push(...pts.filter(([temp]) => temp >= lo && temp <= hi));
  if (pts[pts.length - 1][0] > hi) line.push([hi, speedAt(pts, hi)]);
  return line;
});
const visiblePoints = computed(() => sorted.value.filter(({ p: [temp] }) => temp >= X_MIN.value && temp <= X_MAX.value));

function pathOf(points: [number, number][]): string {
  return points.map(([temp, speed]) => `L ${xToPx(temp)} ${yToPx(speed)}`).join(" ");
}

// The daemon holds the first and last speeds outside the curve, so the fill extends to both axes.
const areaPath = computed(() => {
  const pts = sorted.value.map((pt) => pt.p);
  if (pts.length === 0) return "";
  const line = pathOf(visibleLine.value);
  const [lo, hi] = [X_MIN.value, X_MAX.value];
  return `M ${xToPx(lo)} ${yToPx(Y_MIN)} L ${xToPx(lo)} ${yToPx(speedAt(pts, lo))} ${line} L ${xToPx(hi)} ${yToPx(speedAt(pts, hi))} L ${xToPx(hi)} ${yToPx(Y_MIN)} Z`;
});

const liveMarker = computed(() => {
  const celsius = props.liveCelsius;
  if (celsius === null || celsius === undefined || !props.modelValue.length) return null;
  const temp = clamp(celsius, X_MIN.value, X_MAX.value);
  const speed = Math.round(speedAt(props.modelValue, celsius));
  const x = xToPx(temp);
  const y = yToPx(speed);
  const labelX = clamp(x, PAD + 56, W - PAD - 56);
  const labelY = y > PAD + 40 ? y - 30 : y + 14;
  const tone = celsius < 60 ? "cool" : celsius < 80 ? "warm" : "hot";
  return { x, y, labelX, labelY, tone, label: t("fans.live.marker", { temp: Math.round(celsius), speed }) };
});

const linePath = computed(() => {
  const line = visibleLine.value;
  if (line.length === 0) return "";
  return `M${pathOf(line).slice(1)}`;
});

const xGrid = computed(() => {
  const arr: number[] = [];
  for (let x = X_MIN.value; x <= X_MAX.value; x += 10) arr.push(x);
  return arr;
});
const yGrid = computed(() => {
  const arr: number[] = [];
  for (let y = Y_MIN; y <= Y_MAX; y += 20) arr.push(y);
  return arr;
});

function svgPoint(evt: MouseEvent): { x: number; y: number } | null {
  const matrix = svgRef.value?.getScreenCTM();
  if (!matrix) return null;
  const point = new DOMPoint(evt.clientX, evt.clientY).matrixTransform(matrix.inverse());
  return { x: point.x, y: point.y };
}

function onCanvasClick(evt: MouseEvent) {
  if (props.disabled || dragIndex.value !== null || !canInsertCurvePoint(props.modelValue)) return;
  const point = svgPoint(evt);
  if (!point) return;
  const { x, y } = point;
  if (x < PAD || x > W - PAD || y < PAD || y > H - PAD) return;
  const temp = clamp(pxToX(x), X_MIN.value, X_MAX.value);
  const speed = clamp(pxToY(y), Y_MIN, Y_MAX);
  const next = [...props.modelValue, [round1(temp), round0(speed)] as [number, number]];
  next.sort((a, b) => a[0] - b[0]);
  emit("update:modelValue", next, null);
}

function onPointMouseDown(evt: MouseEvent, index: number) {
  if (props.disabled) return;
  evt.stopPropagation();
  dragIndex.value = index;
  dragRange.value = xRange.value;
  dragGesture = newCurveGesture();
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(evt: MouseEvent) {
  if (dragIndex.value === null) return;
  const point = svgPoint(evt);
  if (!point) return;
  const temp = clamp(pxToX(point.x), X_MIN.value, X_MAX.value);
  const speed = clamp(pxToY(point.y), Y_MIN, Y_MAX);
  const next = props.modelValue.map((p, i) =>
    i === dragIndex.value ? ([round1(temp), round0(speed)] as [number, number]) : p,
  );
  next.sort((a, b) => a[0] - b[0]);
  emit("update:modelValue", next, dragGesture);
}

function onWindowMouseUp() {
  dragIndex.value = null;
  dragRange.value = null;
  dragGesture = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

onUnmounted(onWindowMouseUp);

function onPointDblClick(evt: MouseEvent, index: number) {
  if (props.disabled) return;
  evt.stopPropagation();
  if (props.modelValue.length <= 1) return; // keep at least one point
  const next = props.modelValue.filter((_, i) => i !== index);
  emit("update:modelValue", next, null);
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
      ref="svgRef"
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
        <text :x="W / 2" :y="H - 4" text-anchor="middle" class="axis-title">{{ t("fans.curveEditor.temperatureAxis") }}</text>
        <text
          :x="12"
          :y="H / 2"
          text-anchor="middle"
          class="axis-title"
          :transform="`rotate(-90 12 ${H / 2})`"
        >
          {{ t("fans.curveEditor.speedAxis") }}
        </text>
      </g>
      <!-- curve -->
      <defs>
        <linearGradient
          id="curve-temp-band"
          gradientUnits="userSpaceOnUse"
          :x1="xToPx(CURVE_TEMP_MIN)"
          :x2="xToPx(CURVE_TEMP_MAX)"
          y1="0"
          y2="0"
        >
          <stop offset="0" stop-color="var(--temp-cool)" />
          <stop offset="0.55" stop-color="var(--temp-warm)" />
          <stop offset="1" stop-color="var(--temp-hot)" />
        </linearGradient>
      </defs>
      <rect :x="PAD" :y="H - PAD + 1" :width="innerW" height="4" rx="2" fill="url(#curve-temp-band)" />
      <path v-if="areaPath" :d="areaPath" class="area" />
      <path :d="linePath" class="curve" v-if="linePath" />
      <!-- points -->
      <g v-if="liveMarker" class="live" :class="`tone-${liveMarker.tone}`">
        <line :x1="liveMarker.x" :x2="liveMarker.x" :y1="PAD" :y2="H - PAD" class="live-line" />
        <line :x1="PAD" :x2="liveMarker.x" :y1="liveMarker.y" :y2="liveMarker.y" class="live-line faint" />
        <circle :cx="liveMarker.x" :cy="liveMarker.y" r="7" class="live-dot" />
        <rect :x="liveMarker.labelX - 52" :y="liveMarker.labelY" width="104" height="22" rx="11" class="live-label-bg" />
        <text :x="liveMarker.labelX" :y="liveMarker.labelY + 15" text-anchor="middle" class="live-label">{{ liveMarker.label }}</text>
      </g>
      <g v-for="pt in visiblePoints" :key="pt.i">
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
            rx="11"
          />
          <text :x="pointPx(pt.p).cx" :y="pointPx(pt.p).cy - 19" text-anchor="middle">
            {{ pt.p[0] }}°C → {{ pt.p[1] }}%
          </text>
        </g>
      </g>
    </svg>
    <div class="hint">
      {{ canInsertCurvePoint(modelValue) ? t("fans.curveEditor.hint") : t("fans.points.limit", { max: MAX_CURVE_POINTS }) }}
    </div>
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
  height: 280px;
  width: auto;
  max-width: 100%;
  margin: 0 auto;
  user-select: none;
}
.svg.disabled {
  opacity: 0.5;
  pointer-events: none;
}
.grid line {
  stroke: var(--md-outline-variant);
  stroke-width: 1;
}
.axis-label text {
  fill: var(--md-on-surface-variant);
  font-size: 10px;
}
.axis-title {
  fill: var(--md-on-surface-variant);
  font-size: 11px;
}
.area {
  fill: var(--md-primary);
  fill-opacity: 0.16;
  pointer-events: none;
}
.curve {
  fill: none;
  stroke: var(--md-primary);
  stroke-width: 3;
  stroke-linejoin: round;
  stroke-linecap: round;
}
.point {
  fill: var(--md-surface-container-low);
  stroke: var(--md-primary);
  stroke-width: 2.5;
  pointer-events: none;
  transition: r 0.1s;
}
.point.hover {
  r: 7;
  fill: var(--md-primary);
}
.point-hit {
  fill: transparent;
  cursor: grab;
}
.point-hit:active {
  cursor: grabbing;
}
.live {
  pointer-events: none;
}
.live.tone-cool { --live: var(--temp-cool); }
.live.tone-warm { --live: var(--temp-warm); }
.live.tone-hot { --live: var(--temp-hot); }
.live-line {
  stroke: var(--live);
  stroke-width: 1.5;
  stroke-dasharray: 4 4;
}
.live-line.faint {
  opacity: 0.6;
}
.live-dot {
  fill: var(--live);
  stroke: var(--md-surface-container-low);
  stroke-width: 3;
}
.live-label-bg {
  fill: var(--md-inverse-surface);
}
.live-label {
  fill: var(--md-inverse-on-surface);
  font-size: 11px;
  font-weight: 600;
}
.tooltip rect {
  fill: var(--md-inverse-surface);
}
.tooltip text {
  fill: var(--md-inverse-on-surface);
  font-size: 11px;
  font-weight: 600;
}
.hint {
  color: var(--text-muted);
  font-size: var(--font-size-xs);
}
</style>
