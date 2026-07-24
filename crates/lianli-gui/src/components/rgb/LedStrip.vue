<script setup lang="ts">
import { computed } from "vue";
import type { RGB } from "@/types";

const props = defineProps<{
  colors: RGB[];
  selected: number[];
  count: number;
}>();

const emit = defineEmits<{
  select: [index: number];
}>();

const leds = computed(() => {
  const arr: { color: RGB; selected: boolean }[] = [];
  for (let i = 0; i < props.count; i++) {
    arr.push({
      color: props.colors[i] ?? [0, 0, 0],
      selected: props.selected.includes(i),
    });
  }
  return arr;
});

function css(c: RGB): string {
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}
</script>

<template>
  <div class="led-strip">
    <button
      v-for="(led, i) in leds"
      :key="i"
      type="button"
      class="led"
      :class="{ selected: led.selected }"
      :style="{ background: css(led.color) }"
      :title="`LED ${i + 1}`"
      @click="emit('select', i)"
    />
  </div>
</template>

<style scoped>
.led-strip {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(22px, 1fr));
  gap: var(--space-1);
  max-width: 520px;
}
.led {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 1px solid var(--border-strong);
  cursor: pointer;
  padding: 0;
  transition: transform 0.08s, box-shadow 0.08s;
}
.led:hover {
  transform: scale(1.12);
}
.led.selected {
  box-shadow: 0 0 0 2px var(--accent);
}
</style>
