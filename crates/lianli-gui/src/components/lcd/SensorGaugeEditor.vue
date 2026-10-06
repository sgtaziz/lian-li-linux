<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { Plus, Trash2 } from "lucide-vue-next";
import type { SensorDescriptor, SensorRange } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useFonts } from "@/composables/useFonts";
import ColorPicker from "@/components/rgb/ColorPicker.vue";

const props = defineProps<{ sensor: SensorDescriptor }>();
const config = useConfigStore();
const dialog = useDialog();
const { load: loadFonts, fontOptions } = useFonts();

onMounted(() => void loadFonts());

const localLabel = ref(props.sensor.label);
const localUnit = ref(props.sensor.unit);
watch(
  () => [props.sensor.label, props.sensor.unit] as const,
  ([label, unit]) => {
    if (label !== localLabel.value) localLabel.value = label;
    if (unit !== localUnit.value) localUnit.value = unit;
  },
);

function commitLabel() {
  if (localLabel.value !== props.sensor.label) patch({ label: localLabel.value });
}
function commitUnit() {
  if (localUnit.value !== props.sensor.unit) patch({ unit: localUnit.value });
}

function patch(p: Partial<SensorDescriptor>) {
  Object.assign(props.sensor, p);
  config.markDirty();
}

// ── Gauge ranges ─────────────────────────────────────────────────────────────
const ranges = computed(() => props.sensor.gauge_ranges);

function onRange(i: number, p: Partial<SensorRange>) {
  const next = ranges.value.map((r, idx) => (idx === i ? { ...r, ...p } : r));
  patch({ gauge_ranges: next });
}
function addRange() {
  const next: SensorRange[] = [
    ...ranges.value,
    { max: 100, color: [255, 0, 0] as [number, number, number], alpha: 255 },
  ];
  patch({ gauge_ranges: next });
}
function removeRange(i: number) {
  dialog.error({
    title: "Remove range?",
    content: `Range ${i + 1} will be removed.`,
    positiveText: "Remove",
    negativeText: "Cancel",
    onPositiveClick: () => {
      patch({ gauge_ranges: ranges.value.filter((_, idx) => idx !== i) });
    },
  });
}
</script>

<template>
  <div class="gauge-editor">
    <div class="grid">
      <div class="field">
        <label class="muted">Label</label>
        <n-input v-model:value="localLabel" @blur="commitLabel" size="small" />
      </div>
      <div class="field">
        <label class="muted">Unit</label>
        <n-input v-model:value="localUnit" @blur="commitUnit" size="small" />
      </div>
      <div class="field">
        <label class="muted">Decimal places</label>
        <n-input-number :value="sensor.decimal_places" :min="0" :max="10" size="small" @update:value="(v) => patch({ decimal_places: v ?? 0 })" />
      </div>
      <div class="field">
        <label class="muted">Font</label>
        <n-select
          :value="sensor.font_path ?? ''"
          :options="fontOptions()"
          size="small"
          filterable
          @update:value="(v: string) => patch({ font_path: v || null })"
        />
      </div>
    </div>

    <n-collapse class="sections">
      <n-collapse-item title="Colors and ranges" name="colors">
        <div class="section-body">
          <div class="grid">
            <div class="field"><label class="muted">Text color</label>
              <ColorPicker :model-value="sensor.text_color" @update:model-value="(v: any) => patch({ text_color: v })" />
            </div>
            <div class="field"><label class="muted">Background</label>
              <ColorPicker :model-value="sensor.background_color" @update:model-value="(v: any) => patch({ background_color: v })" />
            </div>
            <div class="field"><label class="muted">Gauge track</label>
              <ColorPicker :model-value="sensor.gauge_background_color" @update:model-value="(v: any) => patch({ gauge_background_color: v })" />
            </div>
          </div>

          <div class="ranges">
            <div class="ranges-head">
              <label class="muted">Gauge colors by value (up to)</label>
              <n-button size="tiny" quaternary @click="addRange"><template #icon><Plus :size="12" /></template>Add</n-button>
            </div>
            <div v-for="(r, i) in ranges" :key="i" class="range-row">
              <n-input-number :value="r.max" size="small" :min="0" placeholder="no limit" class="range-max" @update:value="(v) => onRange(i, { max: v })" />
              <ColorPicker class="range-color" :model-value="[r.color[0], r.color[1], r.color[2], r.alpha ?? 255]" alpha @update:model-value="(v: any) => onRange(i, { color: [v[0], v[1], v[2]], alpha: v[3] ?? 255 })" />
              <n-button size="tiny" quaternary type="error" @click="removeRange(i)"><template #icon><Trash2 :size="12" /></template></n-button>
            </div>
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item title="Text size and position" name="text">
        <div class="section-body">
          <div class="grid">
            <div class="field"><label class="muted">Value size</label>
              <n-input-number :value="sensor.value_font_size" size="small" :min="1" @update:value="(v) => patch({ value_font_size: v ?? 72 })" />
            </div>
            <div class="field"><label class="muted">Unit size</label>
              <n-input-number :value="sensor.unit_font_size" size="small" :min="1" @update:value="(v) => patch({ unit_font_size: v ?? 32 })" />
            </div>
            <div class="field"><label class="muted">Label size</label>
              <n-input-number :value="sensor.label_font_size" size="small" :min="1" @update:value="(v) => patch({ label_font_size: v ?? 28 })" />
            </div>
          </div>
          <div class="grid">
            <div class="field"><label class="muted">Value offset</label>
              <n-input-number :value="sensor.value_offset" size="small" @update:value="(v) => patch({ value_offset: v ?? 0 })" />
            </div>
            <div class="field"><label class="muted">Unit offset</label>
              <n-input-number :value="sensor.unit_offset" size="small" @update:value="(v) => patch({ unit_offset: v ?? 60 })" />
            </div>
            <div class="field"><label class="muted">Label offset</label>
              <n-input-number :value="sensor.label_offset" size="small" @update:value="(v) => patch({ label_offset: v ?? -60 })" />
            </div>
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item title="Gauge shape" name="shape">
        <div class="grid section-body">
          <div class="field"><label class="muted">Start angle</label>
            <n-input-number :value="sensor.gauge_start_angle" size="small" @update:value="(v) => patch({ gauge_start_angle: v ?? 90 })" />
          </div>
          <div class="field"><label class="muted">Sweep angle</label>
            <n-input-number :value="sensor.gauge_sweep_angle" size="small" :min="1" :max="360" @update:value="(v) => patch({ gauge_sweep_angle: v ?? 330 })" />
          </div>
          <div class="field"><label class="muted">Outer radius</label>
            <n-input-number :value="sensor.gauge_outer_radius" size="small" :min="1" @update:value="(v) => patch({ gauge_outer_radius: v ?? 180 })" />
          </div>
          <div class="field"><label class="muted">Thickness</label>
            <n-input-number :value="sensor.gauge_thickness" size="small" :min="1" @update:value="(v) => patch({ gauge_thickness: v ?? 40 })" />
          </div>
          <div class="field"><label class="muted">Corner radius</label>
            <n-input-number :value="sensor.bar_corner_radius" size="small" :min="0" @update:value="(v) => patch({ bar_corner_radius: v ?? 0 })" />
          </div>
        </div>
      </n-collapse-item>
    </n-collapse>
  </div>
</template>

<style scoped>
.gauge-editor {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: var(--space-2);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.sections {
  border-top: 1px solid var(--border);
  padding-top: var(--space-2);
}
.section-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding-bottom: var(--space-2);
}
.grid.section-body {
  display: grid;
}
.ranges {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.ranges-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.range-row {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
}
.range-max {
  width: 130px;
}
/* Give the color picker a definite width so its swatch renders (in a flex row
   it otherwise collapses to just the hex text). */
.range-row .range-color {
  min-width: 130px;
  flex: 1;
}
</style>
