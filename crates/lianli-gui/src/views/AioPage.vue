<script setup lang="ts">
import { computed } from "vue";
import { useDevicesStore } from "@/stores/devices";
import AioCard from "@/components/aio/AioCard.vue";

const devices = useDevicesStore();
const aios = computed(() => devices.aioDevices);
</script>

<template>
  <div class="page aio-page">
    <AioCard v-for="d in aios" :key="d.device_id" :device="d" />
    <div v-if="!aios.length" class="card empty-state muted">
      No AIO devices detected.
    </div>
  </div>
</template>

<style scoped>
.aio-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 1100px;
}
.empty-state {
  padding: var(--space-6);
  text-align: center;
}
</style>
