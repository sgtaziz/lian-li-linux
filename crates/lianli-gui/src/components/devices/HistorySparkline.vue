<script setup lang="ts">
import { computed } from "vue";
import { HISTORY_WINDOW_MS, type Sample } from "@/utils/telemetryHistory";
import { useTranslate } from "@/i18n";

const props = defineProps<{
  samples: Sample[];
  current: number | null;
  label: string;
  unit: string;
  digits: number;
  tone: "rpm" | "coolant";
}>();

const t = useTranslate();
const WIDTH = 300;
const HEIGHT = 48;

const stats = computed(() => {
  const values = props.samples.map((sample) => sample.value);
  if (!values.length) return null;
  return { min: Math.min(...values), max: Math.max(...values) };
});

const paths = computed(() => {
  if (props.samples.length < 2 || !stats.value) return null;
  const end = props.samples[props.samples.length - 1].at;
  const start = Math.max(end - HISTORY_WINDOW_MS, props.samples[0].at);
  const duration = end - start || 1;
  const span = stats.value.max - stats.value.min || 1;
  const points = props.samples.map((sample) => {
    const x = ((sample.at - start) / duration) * WIDTH;
    const y = HEIGHT - 3 - ((sample.value - stats.value!.min) / span) * (HEIGHT - 8);
    return `${x.toFixed(1)} ${y.toFixed(1)}`;
  });
  const line = `M ${points.join(" L ")}`;
  const firstX = points[0].split(" ")[0];
  return { line, area: `${line} L ${WIDTH} ${HEIGHT} L ${firstX} ${HEIGHT} Z` };
});

function format(value: number): string {
  return `${value.toFixed(props.digits)}${props.unit}`;
}

const range = computed(() => (stats.value ? t("devices.history.range", { min: format(stats.value.min), max: format(stats.value.max) }) : ""));
const ariaLabel = computed(() =>
  stats.value ? t("devices.history.summary", { label: props.label, min: format(stats.value.min), max: format(stats.value.max) }) : props.label,
);
</script>

<template>
  <div class="history" :class="`tone-${tone}`">
    <div class="history-head">
      <span class="history-label">{{ label }}</span>
      <span v-if="current !== null" class="mono history-now">{{ format(current) }}</span>
      <span v-else class="history-stale">{{ t("devices.history.noReading") }}</span>
    </div>
    <svg v-if="paths" class="history-chart" :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" preserveAspectRatio="none" role="img" :aria-label="ariaLabel">
      <path :d="paths.area" class="area" />
      <path :d="paths.line" class="line" vector-effect="non-scaling-stroke" />
    </svg>
    <div v-else class="history-chart collecting">{{ t("devices.history.collecting") }}</div>
    <div class="history-foot mono">{{ range }}</div>
  </div>
</template>

<style scoped>
.history {
  --tone: var(--md-primary);
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  border-radius: 16px;
  background: var(--md-surface-container-high);
}
.history.tone-coolant {
  --tone: var(--temp-warm);
}
.history-head {
  display: flex;
  justify-content: space-between;
  gap: var(--space-2);
}
.history-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.history-now {
  font-size: 14px;
  font-weight: 600;
  color: var(--tone);
}
.history-stale {
  font-size: 12px;
  color: var(--md-on-surface-variant);
}
.history-chart {
  display: block;
  width: 100%;
  height: 48px;
}
.collecting {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}
.area {
  fill: var(--tone);
  fill-opacity: 0.14;
}
.line {
  fill: none;
  stroke: var(--tone);
  stroke-width: 2;
  stroke-linejoin: round;
}
.history-foot {
  text-align: right;
  white-space: nowrap;
  font-size: 11px;
  color: var(--md-on-surface-variant);
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
</style>
