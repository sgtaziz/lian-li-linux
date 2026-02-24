<script setup lang="ts">
import type { DeviceInfo } from "../types";
import { FAMILY_NAMES } from "../types";
import { useDeviceStore } from "../stores/devices";
import { computed } from "vue";

const props = defineProps<{
  device: DeviceInfo;
}>();

const deviceStore = useDeviceStore();

const familyName = computed(() => FAMILY_NAMES[props.device.family] ?? props.device.family);
const rpms = computed(() => deviceStore.telemetry.fan_rpms[props.device.device_id] ?? []);
const temp = computed(() => deviceStore.telemetry.coolant_temps[props.device.device_id]);
</script>

<template>
  <div class="rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4 transition-colors hover:border-gray-300 dark:hover:border-gray-600 hover:shadow-sm">
    <div class="flex items-start justify-between">
      <div>
        <h3 class="font-semibold text-sm">{{ familyName }}</h3>
        <p class="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{{ device.name }}</p>
      </div>
      <div class="flex gap-1">
        <span
          v-if="device.has_lcd"
          class="px-1.5 py-0.5 text-xs rounded bg-purple-100 dark:bg-purple-900/40 text-purple-700 dark:text-purple-300"
        >
          LCD
        </span>
        <span
          v-if="device.has_fan"
          class="px-1.5 py-0.5 text-xs rounded bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300"
        >
          Fan
        </span>
        <span
          v-if="device.has_pump"
          class="px-1.5 py-0.5 text-xs rounded bg-teal-100 dark:bg-teal-900/40 text-teal-700 dark:text-teal-300"
        >
          Pump
        </span>
        <span
          v-if="device.has_rgb"
          class="px-1.5 py-0.5 text-xs rounded bg-pink-100 dark:bg-pink-900/40 text-pink-700 dark:text-pink-300"
        >
          RGB
        </span>
      </div>
    </div>

    <div class="mt-3 space-y-1.5 text-xs text-gray-600 dark:text-gray-400">
      <div v-if="device.serial" class="flex justify-between">
        <span>Serial</span>
        <span class="font-mono">{{ device.serial }}</span>
      </div>
      <div v-if="device.screen_width" class="flex justify-between">
        <span>Resolution</span>
        <span>{{ device.screen_width }}x{{ device.screen_height }}</span>
      </div>
      <div v-if="rpms.length > 0" class="flex justify-between">
        <span>Fan RPM</span>
        <span class="font-mono">{{ rpms.join(", ") }}</span>
      </div>
      <div v-if="temp != null" class="flex justify-between">
        <span>Coolant</span>
        <span class="font-mono">{{ temp.toFixed(1) }}°C</span>
      </div>
    </div>
  </div>
</template>
