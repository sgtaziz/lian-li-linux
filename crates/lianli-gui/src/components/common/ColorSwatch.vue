<script setup lang="ts">
import { computed } from "vue";
import type { RGB } from "@/types";

const props = defineProps<{ color: RGB | RGBA | null; size?: number }>();

type RGBA = [number, number, number, number];

const css = computed(() => {
  const c = props.color;
  if (!c) return "transparent";
  if (c.length === 4) {
    return `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${(c[3] / 255).toFixed(3)})`;
  }
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
});
</script>

<template>
  <span
    class="swatch"
    :style="{ background: css, width: (size ?? 20) + 'px', height: (size ?? 20) + 'px' }"
  />
</template>

<style scoped>
.swatch {
  display: inline-block;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-strong);
  flex-shrink: 0;
}
</style>
