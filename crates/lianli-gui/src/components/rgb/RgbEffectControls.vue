<script setup lang="ts">
import { computed } from "vue";
import { Plus, Trash2 } from "lucide-vue-next";
import type { RGB, RGBA, RgbDirection, RgbEffect, RgbEffectParameters } from "@/types";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { RGB_BRIGHTNESS, RGB_DIRECTIONS } from "@/constants";

const props = defineProps<{ effect: RgbEffect; parameters: RgbEffectParameters }>();
const emit = defineEmits<{ update: [patch: Partial<RgbEffect>] }>();

const directionOptions = computed(() => RGB_DIRECTIONS.filter((option) =>
  props.parameters.directions.includes(option.value),
));
const paletteColors = computed(() => {
  const colors = props.effect.colors.slice(0, props.parameters.max_colors);
  const fallback: RGB = colors[colors.length - 1] ?? [255, 255, 255];
  while (colors.length < props.parameters.min_colors) colors.push([...fallback]);
  return colors;
});

function setColor(index: number, value: RGB | RGBA) {
  const colors = [...paletteColors.value];
  colors[index] = [value[0], value[1], value[2]];
  emit("update", { colors });
}

function addColor() {
  if (props.effect.colors.length >= props.parameters.max_colors) return;
  emit("update", { colors: [...props.effect.colors, [255, 255, 255]] });
}

function removeColor(index: number) {
  if (props.effect.colors.length <= props.parameters.min_colors) return;
  emit("update", { colors: props.effect.colors.filter((_, colorIndex) => colorIndex !== index) });
}
</script>

<template>
  <div class="effect-settings">
    <div v-if="parameters.max_colors > 0" class="field">
      <label class="muted">Colors</label>
      <div class="color-list">
        <div v-for="(_, index) in paletteColors" :key="index" class="color-item">
          <ColorPicker class="effect-color" :model-value="paletteColors[index]" @update:model-value="(value) => setColor(index, value)" />
          <n-button v-if="effect.colors.length > parameters.min_colors" quaternary size="small" type="error" @click="removeColor(index)">
            <template #icon><Trash2 :size="14" /></template>
          </n-button>
        </div>
        <n-button v-if="effect.colors.length < parameters.max_colors" quaternary size="small" @click="addColor">
          <template #icon><Plus :size="14" /></template>
        </n-button>
      </div>
    </div>
    <div class="two-col">
      <LabeledSlider v-if="parameters.supports_speed" label="Speed" :model-value="effect.speed" :min="0" :max="4" @update:model-value="(value) => emit('update', { speed: value })" />
      <div class="field">
        <label class="muted">Brightness</label>
        <n-select :value="effect.brightness" :options="RGB_BRIGHTNESS" size="small" @update:value="(value) => emit('update', { brightness: value })" />
      </div>
    </div>
    <div v-if="directionOptions.length" class="field">
      <label class="muted">Direction</label>
      <n-select :value="effect.direction" :options="directionOptions" size="small" @update:value="(value: RgbDirection) => emit('update', { direction: value })" />
    </div>
  </div>
</template>

<style scoped>
.effect-settings, .field { display: flex; flex-direction: column; gap: var(--space-3); }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); }
.color-list, .color-item { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
.effect-color { min-width: 130px; }
@media (max-width: 700px) { .two-col { grid-template-columns: 1fr; } }
</style>
