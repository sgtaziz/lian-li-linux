<script setup lang="ts">
import { computed } from "vue";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import RgbDeviceCard from "@/components/rgb/RgbDeviceCard.vue";
import StatusDot from "@/components/common/StatusDot.vue";

const daemon = useDaemonStore();
const config = useConfigStore();

const rgb = computed(() => config.ensureRgb());

const openrgbEnabled = computed({
  get: () => rgb.value.openrgb_server,
  set: (v: boolean) => {
    rgb.value.openrgb_server = v;
    // OpenRGB toggles auto-save (matching Slint behaviour).
    void config.save();
  },
});

const openrgbPort = computed({
  get: () => rgb.value.openrgb_port,
  set: (v: number) => {
    rgb.value.openrgb_port = v;
    config.markDirty();
  },
});

const statusText = computed(() => {
  if (!openrgbEnabled.value) return "Disabled";
  if (daemon.openrgbError) return `Error: ${daemon.openrgbError}`;
  if (daemon.openrgbRunning) return `Listening on port ${daemon.openrgbPort ?? rgb.value.openrgb_port}`;
  return "Starting...";
});

const rgbCaps = computed(() => config.rgbCaps);
</script>

<template>
  <div class="page rgb-page">
    <!-- OpenRGB status card -->
    <section class="card openrgb">
      <div class="openrgb-head">
        <h2 class="section-title">OpenRGB</h2>
        <span class="status-tag">
          <StatusDot
            :color="openrgbEnabled ? (daemon.openrgbRunning ? 'success' : 'warning') : 'muted'"
          />
          {{ statusText }}
        </span>
      </div>
      <div class="openrgb-body">
        <n-checkbox v-model:checked="openrgbEnabled">Enable OpenRGB SDK server</n-checkbox>
        <div class="port-row" v-if="openrgbEnabled">
          <label class="muted">Port</label>
          <n-input-number
            v-model:value="openrgbPort"
            size="small"
            :min="1"
            :max="65535"
          />
        </div>
        <p class="hint" v-if="openrgbEnabled">
          The OpenRGB SDK server binds to a TCP port without authentication. Any local process
          can control your RGB devices. While enabled, the daemon will not apply its own RGB effects.
        </p>
      </div>
    </section>

    <!-- Per-device RGB cards -->
    <RgbDeviceCard v-for="cap in rgbCaps" :key="cap.device_id" :cap="cap" />

    <div v-if="!rgbCaps.length" class="card empty-state muted">
      No RGB-capable devices detected.
    </div>
  </div>
</template>

<style scoped>
.rgb-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 1100px;
}
.openrgb {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.openrgb-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}
.openrgb-head .section-title {
  margin: 0;
}
.status-tag {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  white-space: nowrap;
}
.openrgb-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.port-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.empty-state {
  padding: var(--space-6);
  text-align: center;
}
</style>
