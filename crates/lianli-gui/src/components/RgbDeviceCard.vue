<script setup lang="ts">
import { ref, computed } from "vue";
import type {
  RgbDeviceCapabilities,
  RgbDeviceConfig,
  RgbEffect,
  RgbMode,
  RgbDirection,
} from "../types";
import { RGB_MODE_NAMES } from "../types";

const props = defineProps<{
  capabilities: RgbDeviceCapabilities;
  deviceConfig: RgbDeviceConfig;
}>();

const emit = defineEmits<{
  (e: "zone-update", deviceId: string, zoneIndex: number, effect: RgbEffect): void;
  (e: "apply-to-all", deviceId: string, effect: RgbEffect): void;
  (e: "mb-rgb-sync", deviceId: string, enabled: boolean): void;
}>();

const expandedZone = ref<number | null>(null);

const availableModes = computed(() =>
  ["Off" as RgbMode, ...props.capabilities.supported_modes].filter(
    (m) => m !== "Direct"
  )
);

const directions: { value: RgbDirection; label: string }[] = [
  { value: "Clockwise", label: "CW" },
  { value: "CounterClockwise", label: "CCW" },
  { value: "Up", label: "Up" },
  { value: "Down", label: "Down" },
  { value: "Spread", label: "Spread" },
  { value: "Gather", label: "Gather" },
];

function effectFor(zoneIndex: number): RgbEffect {
  const zone = props.deviceConfig.zones.find(
    (z) => z.zone_index === zoneIndex
  );
  return (
    zone?.effect ?? {
      mode: "Static",
      colors: [[255, 255, 255]],
      speed: 2,
      brightness: 4,
      direction: "Clockwise",
      scope: "All",
    }
  );
}

function toggleZone(idx: number) {
  expandedZone.value = expandedZone.value === idx ? null : idx;
}

function updateMode(zoneIndex: number, mode: RgbMode) {
  const effect = { ...effectFor(zoneIndex), mode };
  if (effect.colors.length === 0 && mode !== "Off") {
    effect.colors = [[255, 255, 255]];
  }
  emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
}

function updateColor(zoneIndex: number, colorIdx: number, hex: string) {
  const effect = { ...effectFor(zoneIndex), colors: [...effectFor(zoneIndex).colors] };
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  effect.colors[colorIdx] = [r, g, b];
  emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
}

function addColor(zoneIndex: number) {
  const effect = { ...effectFor(zoneIndex), colors: [...effectFor(zoneIndex).colors] };
  if (effect.colors.length < 4) {
    effect.colors.push([255, 255, 255]);
    emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
  }
}

function removeColor(zoneIndex: number, colorIdx: number) {
  const effect = { ...effectFor(zoneIndex), colors: [...effectFor(zoneIndex).colors] };
  if (effect.colors.length > 1) {
    effect.colors.splice(colorIdx, 1);
    emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
  }
}

function updateSpeed(zoneIndex: number, speed: number) {
  const effect = { ...effectFor(zoneIndex), speed };
  emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
}

function updateBrightness(zoneIndex: number, brightness: number) {
  const effect = { ...effectFor(zoneIndex), brightness };
  emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
}

function updateDirection(zoneIndex: number, direction: RgbDirection) {
  const effect = { ...effectFor(zoneIndex), direction };
  emit("zone-update", props.capabilities.device_id, zoneIndex, effect);
}

function applyToAll() {
  if (expandedZone.value !== null) {
    emit(
      "apply-to-all",
      props.capabilities.device_id,
      effectFor(expandedZone.value)
    );
  }
}

function rgbToHex(color: [number, number, number]): string {
  return (
    "#" +
    color
      .map((c) => c.toString(16).padStart(2, "0"))
      .join("")
  );
}
</script>

<template>
  <div
    class="rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800"
  >
    <!-- Header -->
    <div class="px-4 py-3 border-b border-gray-100 dark:border-gray-700">
      <div class="flex items-center justify-between">
        <div>
          <span class="text-sm font-semibold">{{
            capabilities.device_name
          }}</span>
          <span class="ml-2 text-xs text-gray-400">
            {{ capabilities.zones.length }} zone(s),
            {{ capabilities.total_led_count }} LEDs
          </span>
        </div>
        <div class="flex items-center gap-2">
          <button
            v-if="capabilities.supports_mb_rgb_sync"
            @click="emit('mb-rgb-sync', capabilities.device_id, !deviceConfig.mb_rgb_sync)"
            class="px-2 py-1 text-xs rounded transition-colors"
            :class="
              deviceConfig.mb_rgb_sync
                ? 'bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300'
                : 'bg-gray-100 dark:bg-gray-700 text-gray-500 hover:bg-gray-200 dark:hover:bg-gray-600'
            "
          >
            MB Sync {{ deviceConfig.mb_rgb_sync ? "On" : "Off" }}
          </button>
          <button
            v-if="expandedZone !== null"
            @click="applyToAll"
            class="px-2 py-1 text-xs rounded bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
          >
            Apply to all zones
          </button>
          <span class="text-xs text-gray-400 font-mono">{{
            capabilities.device_id
          }}</span>
        </div>
      </div>
    </div>

    <!-- Zones -->
    <div class="divide-y divide-gray-100 dark:divide-gray-700">
      <div
        v-for="(zone, idx) in capabilities.zones"
        :key="idx"
      >
        <!-- Zone header (clickable) -->
        <button
          @click="toggleZone(idx)"
          class="w-full px-4 py-2.5 flex items-center justify-between text-left hover:bg-gray-50 dark:hover:bg-gray-750 transition-colors"
        >
          <div class="flex items-center gap-3">
            <span class="text-sm">{{ zone.name }}</span>
            <span class="text-xs text-gray-400">{{ zone.led_count }} LEDs</span>
          </div>
          <div class="flex items-center gap-2">
            <!-- Current color preview -->
            <div class="flex gap-0.5">
              <div
                v-for="(color, ci) in effectFor(idx).colors.slice(0, 4)"
                :key="ci"
                class="w-3 h-3 rounded-sm border border-gray-300 dark:border-gray-600"
                :style="{ backgroundColor: rgbToHex(color) }"
              />
            </div>
            <span class="text-xs text-gray-500">{{
              RGB_MODE_NAMES[effectFor(idx).mode] ?? effectFor(idx).mode
            }}</span>
            <span
              class="text-gray-400 transition-transform"
              :class="expandedZone === idx ? 'rotate-90' : ''"
            >
              &#9654;
            </span>
          </div>
        </button>

        <!-- Zone controls (expanded) -->
        <div
          v-if="expandedZone === idx"
          class="px-4 pb-4 pt-2 bg-gray-50/50 dark:bg-gray-800/50 space-y-3"
        >
          <!-- Mode -->
          <div>
            <label class="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Mode
            </label>
            <select
              :value="effectFor(idx).mode"
              @change="updateMode(idx, ($event.target as HTMLSelectElement).value as RgbMode)"
              class="w-full px-2 py-1.5 text-sm rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700"
            >
              <option v-for="mode in availableModes" :key="mode" :value="mode">
                {{ RGB_MODE_NAMES[mode] ?? mode }}
              </option>
            </select>
          </div>

          <!-- Colors -->
          <div v-if="effectFor(idx).mode !== 'Off'">
            <label class="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Colors
            </label>
            <div class="flex items-center gap-2 flex-wrap">
              <div
                v-for="(color, ci) in effectFor(idx).colors"
                :key="ci"
                class="flex items-center gap-1"
              >
                <input
                  type="color"
                  :value="rgbToHex(color)"
                  @input="updateColor(idx, ci, ($event.target as HTMLInputElement).value)"
                  class="w-8 h-8 rounded border border-gray-300 dark:border-gray-600 cursor-pointer"
                />
                <button
                  v-if="effectFor(idx).colors.length > 1"
                  @click="removeColor(idx, ci)"
                  class="text-xs text-gray-400 hover:text-red-500"
                >
                  x
                </button>
              </div>
              <button
                v-if="effectFor(idx).colors.length < 4"
                @click="addColor(idx)"
                class="w-8 h-8 rounded border border-dashed border-gray-300 dark:border-gray-600 text-gray-400 hover:text-gray-600 text-sm flex items-center justify-center"
              >
                +
              </button>
            </div>
          </div>

          <!-- Speed -->
          <div v-if="effectFor(idx).mode !== 'Off' && effectFor(idx).mode !== 'Static'">
            <label class="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Speed: {{ effectFor(idx).speed }}
            </label>
            <input
              type="range"
              min="0"
              max="4"
              :value="effectFor(idx).speed"
              @input="updateSpeed(idx, parseInt(($event.target as HTMLInputElement).value))"
              class="w-full h-1.5 accent-blue-500"
            />
          </div>

          <!-- Brightness -->
          <div v-if="effectFor(idx).mode !== 'Off'">
            <label class="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Brightness: {{ effectFor(idx).brightness }}
            </label>
            <input
              type="range"
              min="0"
              max="4"
              :value="effectFor(idx).brightness"
              @input="updateBrightness(idx, parseInt(($event.target as HTMLInputElement).value))"
              class="w-full h-1.5 accent-blue-500"
            />
          </div>

          <!-- Direction -->
          <div v-if="effectFor(idx).mode !== 'Off' && effectFor(idx).mode !== 'Static'">
            <label class="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Direction
            </label>
            <div class="flex gap-1 flex-wrap">
              <button
                v-for="dir in directions"
                :key="dir.value"
                @click="updateDirection(idx, dir.value)"
                class="px-2 py-1 text-xs rounded transition-colors"
                :class="
                  effectFor(idx).direction === dir.value
                    ? 'bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300'
                    : 'bg-gray-100 dark:bg-gray-700 text-gray-500 hover:bg-gray-200 dark:hover:bg-gray-600'
                "
              >
                {{ dir.label }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
