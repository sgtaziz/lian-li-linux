<script setup lang="ts">
import { computed } from "vue";
import { useDevicesStore } from "@/stores/devices";
import AioCard from "@/components/aio/AioCard.vue";
import EmptyState from "@/components/common/EmptyState.vue";

const devices = useDevicesStore();
const aios = computed(() => devices.aioDevices);
</script>

<template>
  <div class="page aio-page">
    <AioCard v-for="d in aios" :key="d.device_id" :device="d" />
    <EmptyState v-if="!aios.length" kind="aio" />
  </div>
</template>

<style scoped>
.aio-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
</style>
