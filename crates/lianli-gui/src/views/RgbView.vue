<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useConfigStore } from "../stores/config";
import { useDeviceStore } from "../stores/devices";
import PageHeader from "../components/PageHeader.vue";
import RgbDeviceCard from "../components/RgbDeviceCard.vue";
import type {
  RgbDeviceCapabilities,
  RgbAppConfig,
  RgbDeviceConfig,
  RgbEffect,
} from "../types";

const configStore = useConfigStore();
const deviceStore = useDeviceStore();

const capabilities = ref<RgbDeviceCapabilities[]>([]);
const loadingCaps = ref(false);

// Get or create RGB config
const rgbConfig = computed<RgbAppConfig>(
  () =>
    configStore.rgbConfig ?? {
      enabled: true,
      openrgb_server: false,
      openrgb_port: 6743,
      devices: [],
    }
);

async function loadCapabilities() {
  if (!deviceStore.daemonConnected) return;
  try {
    loadingCaps.value = true;
    capabilities.value = await invoke<RgbDeviceCapabilities[]>(
      "get_rgb_capabilities"
    );
  } catch (e) {
    console.error("Failed to load RGB capabilities:", e);
  } finally {
    loadingCaps.value = false;
  }
}

function capsFor(deviceId: string): RgbDeviceCapabilities | undefined {
  return capabilities.value.find((c) => c.device_id === deviceId);
}

function deviceConfigFor(deviceId: string): RgbDeviceConfig {
  const existing = rgbConfig.value.devices.find(
    (d) => d.device_id === deviceId
  );
  if (existing) return existing;
  return { device_id: deviceId, mb_rgb_sync: false, zones: [] };
}

function handleZoneUpdate(
  deviceId: string,
  zoneIndex: number,
  effect: RgbEffect
) {
  const cfg = { ...rgbConfig.value };
  let devCfg = cfg.devices.find((d) => d.device_id === deviceId);
  if (!devCfg) {
    devCfg = { device_id: deviceId, mb_rgb_sync: false, zones: [] };
    cfg.devices.push(devCfg);
  }

  let zoneCfg = devCfg.zones.find((z) => z.zone_index === zoneIndex);
  if (!zoneCfg) {
    zoneCfg = {
      zone_index: zoneIndex,
      effect,
      swap_lr: false,
      swap_tb: false,
    };
    devCfg.zones.push(zoneCfg);
  } else {
    zoneCfg.effect = effect;
  }

  configStore.updateRgbConfig(cfg);

  // Also send immediate effect via IPC
  invoke("set_rgb_effect", {
    deviceId,
    zone: zoneIndex,
    effect,
  }).catch((e: unknown) => console.error("Failed to set RGB effect:", e));
}

function handleApplyToAll(deviceId: string, effect: RgbEffect) {
  const cap = capsFor(deviceId);
  if (!cap) return;
  for (let i = 0; i < cap.zones.length; i++) {
    handleZoneUpdate(deviceId, i, effect);
  }
}

function handleMbRgbSync(deviceId: string, enabled: boolean) {
  const cfg = { ...rgbConfig.value };
  let devCfg = cfg.devices.find((d) => d.device_id === deviceId);
  if (!devCfg) {
    devCfg = { device_id: deviceId, mb_rgb_sync: enabled, zones: [] };
    cfg.devices.push(devCfg);
  } else {
    devCfg.mb_rgb_sync = enabled;
  }
  configStore.updateRgbConfig(cfg);

  invoke("set_mb_rgb_sync", { deviceId, enabled }).catch((e: unknown) =>
    console.error("Failed to set MB RGB sync:", e)
  );
}

function toggleEnabled() {
  const cfg = { ...rgbConfig.value, enabled: !rgbConfig.value.enabled };
  configStore.updateRgbConfig(cfg);
}

function toggleOpenRgb() {
  const cfg = {
    ...rgbConfig.value,
    openrgb_server: !rgbConfig.value.openrgb_server,
  };
  configStore.updateRgbConfig(cfg);
}

onMounted(loadCapabilities);
watch(() => deviceStore.daemonConnected, (connected) => {
  if (connected) loadCapabilities();
});
</script>

<template>
  <div>
    <PageHeader title="RGB Effects">
      <template #actions>
        <button
          @click="toggleEnabled"
          class="px-3 py-1.5 text-sm rounded-lg transition-colors"
          :class="
            rgbConfig.enabled
              ? 'bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300'
              : 'bg-gray-100 dark:bg-gray-700 text-gray-500'
          "
        >
          {{ rgbConfig.enabled ? "Enabled" : "Disabled" }}
        </button>
        <button
          @click="loadCapabilities"
          :disabled="loadingCaps"
          class="px-3 py-1.5 text-sm rounded-lg bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
        >
          Refresh
        </button>
        <button
          @click="configStore.save()"
          :disabled="!configStore.dirty || configStore.loading"
          class="px-4 py-1.5 text-sm rounded-lg font-medium transition-colors"
          :class="
            configStore.dirty
              ? 'bg-blue-500 text-white hover:bg-blue-600'
              : 'bg-gray-200 dark:bg-gray-700 text-gray-400 cursor-not-allowed'
          "
        >
          {{ configStore.loading ? "Saving..." : "Save" }}
        </button>
      </template>
    </PageHeader>

    <div v-if="configStore.error" class="mb-4 text-sm text-red-500">
      {{ configStore.error }}
    </div>

    <!-- OpenRGB server toggle -->
    <div
      class="mb-4 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4"
    >
      <div class="flex items-center justify-between">
        <div>
          <span class="text-sm font-semibold">OpenRGB SDK Server</span>
          <p class="text-xs text-gray-400 mt-0.5">
            Expose devices to OpenRGB / SignalRGB on port
            {{ rgbConfig.openrgb_port }}
          </p>
        </div>
        <button
          @click="toggleOpenRgb"
          class="px-3 py-1 text-xs rounded-lg transition-colors"
          :class="
            rgbConfig.openrgb_server
              ? 'bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300'
              : 'bg-gray-100 dark:bg-gray-700 text-gray-500'
          "
        >
          {{ rgbConfig.openrgb_server ? "Running" : "Stopped" }}
        </button>
      </div>
      <div
        v-if="rgbConfig.openrgb_server"
        class="mt-3 pt-3 border-t border-gray-100 dark:border-gray-700"
      >
        <p class="text-xs font-medium text-gray-500 dark:text-gray-400 mb-1.5">
          How to connect:
        </p>
        <ol class="text-xs text-gray-400 space-y-1 list-decimal list-inside">
          <li>
            Open <span class="font-medium text-gray-600 dark:text-gray-300">OpenRGB</span>
            &rarr; Settings &rarr; <span class="font-medium text-gray-600 dark:text-gray-300">SDK Client</span>
          </li>
          <li>
            Add server: <code class="px-1 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 font-mono">localhost:{{ rgbConfig.openrgb_port }}</code>
          </li>
          <li>Click Connect &mdash; your Lian Li devices will appear as controllers</li>
        </ol>
        <p class="text-xs text-gray-400 mt-2">
          While OpenRGB is connected, it takes priority over effects set here.
        </p>
      </div>
    </div>

    <div v-if="!configStore.config" class="text-sm text-gray-500">
      No config loaded. Is the daemon running?
    </div>

    <div v-else-if="capabilities.length === 0 && !loadingCaps" class="text-center py-12">
      <p class="text-gray-500 dark:text-gray-400 text-sm">
        No RGB devices detected.
      </p>
      <p class="text-gray-400 dark:text-gray-500 text-xs mt-1">
        Connect a Lian Li fan controller or wireless fans to get started.
      </p>
    </div>

    <div v-else-if="loadingCaps" class="text-center py-12">
      <p class="text-gray-500 dark:text-gray-400 text-sm">
        Loading RGB devices...
      </p>
    </div>

    <div v-else class="space-y-4">
      <RgbDeviceCard
        v-for="cap in capabilities"
        :key="cap.device_id"
        :capabilities="cap"
        :device-config="deviceConfigFor(cap.device_id)"
        @zone-update="handleZoneUpdate"
        @apply-to-all="handleApplyToAll"
        @mb-rgb-sync="handleMbRgbSync"
      />
    </div>
  </div>
</template>
