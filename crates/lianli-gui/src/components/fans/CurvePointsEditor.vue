<script setup lang="ts">
import { computed } from "vue";
import { Plus, X } from "lucide-vue-next";
import NumberField from "@/components/common/NumberField.vue";
import { useTranslate } from "@/i18n";
import {
  CURVE_POINT_TEMP_MAX,
  CURVE_POINT_TEMP_MIN,
  MAX_CURVE_POINTS,
  canInsertCurvePoint,
  insertCurvePoint,
  sortCurve,
} from "./curvePoints";

const props = defineProps<{
  modelValue: [number, number][];
  disabled?: boolean;
}>();

const emit = defineEmits<{ "update:modelValue": [value: [number, number][], gesture: number | null] }>();

const t = useTranslate();

const canAdd = computed(() => canInsertCurvePoint(props.modelValue));

const duplicateTemps = computed(() => {
  const counts = new Map<number, number>();
  for (const [temp] of props.modelValue) counts.set(temp, (counts.get(temp) ?? 0) + 1);
  return new Set([...counts].filter(([, count]) => count > 1).map(([temp]) => temp));
});

function setValue(index: number, axis: 0 | 1, value: number) {
  const next = props.modelValue.map((point) => [...point] as [number, number]);
  next[index][axis] = value;
  emit("update:modelValue", next, null);
}

function commitOrder() {
  const sorted = sortCurve(props.modelValue);
  if (sorted.some((point, index) => point !== props.modelValue[index])) emit("update:modelValue", sorted, null);
}

function remove(index: number) {
  if (props.modelValue.length <= 1) return;
  emit("update:modelValue", props.modelValue.filter((_, i) => i !== index), null);
}

function add() {
  if (canAdd.value) emit("update:modelValue", insertCurvePoint(props.modelValue), null);
}
</script>

<template>
  <div class="points">
    <div class="points-head">
      <span class="points-title">{{ t("fans.points.title") }}</span>
      <span class="points-hint">{{ t("fans.points.hint") }}</span>
    </div>
    <div class="grid">
      <div
        v-for="(point, index) in modelValue"
        :key="index"
        class="point"
        :class="{ duplicate: duplicateTemps.has(point[0]) }"
      >
        <div class="point-head">
          <span class="point-name">{{ t("fans.points.point", { index: index + 1 }) }}</span>
          <button
            v-if="modelValue.length > 1"
            type="button"
            class="remove"
            :disabled="disabled"
            :aria-label="t('fans.points.remove', { index: index + 1 })"
            :title="t('fans.points.remove', { index: index + 1 })"
            @click="remove(index)"
          >
            <X :size="14" :stroke-width="2.4" />
          </button>
        </div>
        <div class="fields">
          <label class="field">
            <span>{{ t("fans.points.temperature") }}</span>
            <NumberField
              :model-value="point[0]"
              :label="t('fans.points.temperatureLabel', { index: index + 1 })"
              :min="CURVE_POINT_TEMP_MIN"
              :max="CURVE_POINT_TEMP_MAX"
              :precision="1"
              unit="°C"
              :invalid="duplicateTemps.has(point[0])"
              :disabled="disabled"
              @update:model-value="(v) => setValue(index, 0, v)"
              @commit="commitOrder"
            />
          </label>
          <label class="field">
            <span>{{ t("fans.points.speed") }}</span>
            <NumberField
              :model-value="point[1]"
              :label="t('fans.points.speedLabel', { index: index + 1 })"
              :min="0"
              :max="100"
              unit="%"
              :disabled="disabled"
              @update:model-value="(v) => setValue(index, 1, v)"
            />
          </label>
        </div>
      </div>
      <button type="button" class="add" :disabled="disabled || !canAdd" @click="add">
        <Plus :size="18" :stroke-width="2.2" />
        {{ t("fans.points.add") }}
      </button>
    </div>
    <p v-if="!canAdd" class="limit">{{ t("fans.points.limit", { max: MAX_CURVE_POINTS }) }}</p>
    <p v-if="duplicateTemps.size" class="warning">{{ t("fans.points.duplicate") }}</p>
  </div>
</template>

<style scoped>
.points {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.points-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.points-title {
  font-size: 15px;
  font-weight: 600;
}
.points-hint {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(216px, 1fr));
  gap: 10px;
}
.point {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: 10px 10px 12px 14px;
  border-radius: 18px;
  border: 1px solid transparent;
  background: var(--md-surface-container-high);
}
.point.duplicate {
  border-color: var(--danger);
}
.point-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 24px;
}
.point-name {
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: 14px;
  background: transparent;
  color: var(--md-on-surface-variant);
  cursor: pointer;
}
.remove:hover:not(:disabled) {
  background: color-mix(in srgb, var(--danger) 16%, transparent);
  color: var(--danger);
}
.fields {
  display: flex;
  gap: var(--space-2);
}
.field {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: var(--font-size-xs);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}
.add {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  min-height: 104px;
  border: 1.5px dashed var(--md-outline);
  border-radius: 18px;
  background: transparent;
  color: var(--md-primary);
  font: inherit;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
}
.add:hover:not(:disabled) {
  background: color-mix(in srgb, var(--md-primary) 8%, transparent);
}
.add:disabled,
.remove:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.warning {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--danger);
}
.limit {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
</style>
