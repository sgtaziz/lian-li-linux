<script setup lang="ts">
import { computed } from "vue";
import { useConfigStore } from "@/stores/config";

const config = useConfigStore();
const enabled = computed({
  get: () => config.ensureRgb().openrgb_regions ?? false,
  set: (value: boolean) => {
    config.ensureRgb().openrgb_regions = value;
    config.markDirty();
  },
});
</script>

<template>
  <div>
    <n-checkbox v-model:checked="enabled">AL V2 inner/outer regions</n-checkbox>
    <p class="hint">
      Give each fan group independent inner and outer lighting effects.
      Save to reconnect OpenRGB, then recreate profiles for the new layout.
    </p>
  </div>
</template>
