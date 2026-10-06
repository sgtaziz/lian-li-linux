<script setup lang="ts">
import { computed } from "vue";
import { useDevicesStore } from "@/stores/devices";
import DeviceCard from "@/components/devices/DeviceCard.vue";
import { PlugZap, RefreshCw } from "lucide-vue-next";
import { useDaemonStore } from "@/stores/daemon";

const devices = useDevicesStore();
const daemon = useDaemonStore();

const empty = computed(() => devices.displayCards.length === 0);

async function refresh() {
  await daemon.refresh();
}
</script>

<template>
  <div class="page devices-page">
    <div class="page-head">
      <span class="muted">{{ devices.summary }}</span>
      <n-button quaternary size="small" @click="refresh">
        <template #icon><RefreshCw :size="15" /></template>
      </n-button>
    </div>

    <n-alert v-for="(error, id) in devices.displaySwitchErrors" :key="id" type="error" closable @close="delete devices.displaySwitchErrors[id]">{{ error }}</n-alert>

    <div v-if="empty" class="empty-state">
      <PlugZap :size="48" :stroke-width="1.4" />
      <div class="empty-title">No devices found</div>
      <div class="empty-sub">Is the daemon running?</div>
      <n-button size="small" @click="refresh">Retry</n-button>
    </div>

    <div v-else class="card device-list">
      <DeviceCard
        v-for="d in devices.displayCards"
        :key="d.device_id"
        :device="d"
      />
    </div>
  </div>
</template>

<style scoped>
.page {
  width: 100%;
  max-width: 1100px;
}
.page-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-4);
}
.device-list {
  container-type: inline-size;
  padding: var(--space-1) 0;
}
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-8);
  color: var(--text-muted);
}
.empty-title {
  font-size: var(--font-size-lg);
  color: var(--text-secondary);
  margin-top: var(--space-2);
}
.empty-sub {
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-2);
}
</style>
