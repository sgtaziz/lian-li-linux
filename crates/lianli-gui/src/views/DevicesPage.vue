<script setup lang="ts">
import { computed } from "vue";
import { useDevicesStore } from "@/stores/devices";
import DeviceCard from "@/components/devices/DeviceCard.vue";
import { PlugZap, RefreshCw } from "lucide-vue-next";
import { useDaemonStore } from "@/stores/daemon";

const devices = useDevicesStore();
const daemon = useDaemonStore();

const empty = computed(() => devices.visible.length === 0);

async function refresh() {
  await daemon.refresh();
}
</script>

<template>
  <div class="page devices-page">
    <div class="page-head">
      <span class="muted">{{ devices.visible.length }} device(s) detected</span>
      <n-button quaternary size="small" @click="refresh">
        <template #icon><RefreshCw :size="15" /></template>
      </n-button>
    </div>

    <div v-if="empty" class="empty-state">
      <PlugZap :size="48" :stroke-width="1.4" />
      <div class="empty-title">No devices found</div>
      <div class="empty-sub">Is the daemon running?</div>
      <n-button size="small" @click="refresh">Retry</n-button>
    </div>

    <div v-else class="grid">
      <DeviceCard
        v-for="d in devices.visible"
        :key="d.device_id"
        :device="d"
      />
    </div>
  </div>
</template>

<style scoped>
/* No max-width: let the auto-fit grid use the full content width so wide
   screens show 4+ cards per row instead of leaving empty space. */
.page {
  width: 100%;
}
.page-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-4);
}
.grid {
  display: grid;
  /* auto-fit collapses empty tracks so cards stretch to fill the row when the
     window is wide enough for more tracks than there are devices. */
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-2);
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
