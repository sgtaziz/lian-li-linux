<script setup lang="ts">
import { computed } from "vue";
import { Plus } from "lucide-vue-next";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import LcdConfigCard from "@/components/lcd/LcdConfigCard.vue";

const config = useConfigStore();
const devices = useDevicesStore();

const entries = computed(() => config.config.lcds);

function addLcd() {
  const first = devices.lcdDevices[0];
  config.addLcd({
    serial: first?.serial ?? null,
    index: first?.serial ? undefined : 0,
    type: "image",
    path: null,
    fps: null,
    orientation: 0,
    rgb: null,
  });
}
</script>

<template>
  <div class="page lcd-page">
    <div class="page-head">
      <n-button size="small" type="primary" @click="addLcd" :disabled="!devices.lcdDevices.length">
        <template #icon><Plus :size="15" /></template>
        Add LCD
      </n-button>
      <span v-if="!devices.lcdDevices.length" class="muted">
        No LCD devices detected.
      </span>
    </div>

    <LcdConfigCard
      v-for="(entry, i) in entries"
      :key="i"
      :entry="entry"
      :index="i"
    />

    <div v-if="!entries.length && devices.lcdDevices.length" class="card empty muted">
      No LCD configurations. Click "Add LCD" to create one.
    </div>
  </div>
</template>

<style scoped>
.lcd-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 1100px;
}
.page-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.empty {
  padding: var(--space-6);
  text-align: center;
}
</style>
