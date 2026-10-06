<script setup lang="ts">
import { computed, ref } from "vue";
import { Plus, Image, Film, Palette, Gauge, LayoutTemplate, MonitorOff } from "lucide-vue-next";
import type { LcdConfig, MediaType } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { resolveLcdDevice } from "@/utils/lcdSelection";
import LcdConfigCard from "@/components/lcd/LcdConfigCard.vue";
import MediaAccessNotice from "@/components/lcd/MediaAccessNotice.vue";
import ManagedMediaImport from "@/components/lcd/ManagedMediaImport.vue";
import StartupImageDialog from "@/components/lcd/StartupImageDialog.vue";

const config = useConfigStore();
const devices = useDevicesStore();

const entries = computed(() => config.config.lcds);
const wirelessImageDevices = computed(() => devices.list.filter(device => device.family === "WirelessAio" && device.startup_image));
const selectedTemplates = computed(() => {
  const ids = new Set(entries.value.filter((entry) => entry.type === "custom").map((entry) => entry.template_id));
  return config.templates.filter((template) => ids.has(template.id));
});

const accessIssues = ref(false);
const importActive = ref(false);
const copiesOpen = ref(false);
const showCopies = computed(() => accessIssues.value || importActive.value || copiesOpen.value);
function onImportProgress(active: boolean) {
  if (importActive.value && !active) copiesOpen.value = true;
  importActive.value = active;
}

const selectedRaw = ref(0);
const selected = computed(() => Math.min(selectedRaw.value, Math.max(0, entries.value.length - 1)));
const current = computed(() => entries.value[selected.value]);

const MEDIA_META: Record<MediaType, { label: string; icon: typeof Image }> = {
  image: { label: "Image", icon: Image },
  video: { label: "Video", icon: Film },
  gif: { label: "GIF", icon: Film },
  color: { label: "Solid color", icon: Palette },
  sensor: { label: "Sensor gauge", icon: Gauge },
  custom: { label: "Template", icon: LayoutTemplate },
  doublegauge: { label: "Legacy", icon: Gauge },
  cooler: { label: "Legacy", icon: Gauge },
};

function mediaMeta(entry: LcdConfig) {
  return MEDIA_META[entry.type] ?? { label: entry.type, icon: Image };
}

function deviceName(entry: LcdConfig): string | null {
  return resolveLcdDevice(entry, devices.lcdDevices)?.name ?? null;
}

function summary(entry: LcdConfig): string {
  switch (entry.type) {
    case "sensor":
      return entry.sensor ? `${entry.sensor.label} ${entry.sensor.unit}`.trim() : "";
    case "custom":
      return config.templates.find((t) => t.id === entry.template_id)?.name ?? "No template";
    case "color": {
      const [r, g, b] = entry.rgb ?? [0, 0, 0];
      return `rgb(${r}, ${g}, ${b})`;
    }
    default:
      return entry.path?.split("/").pop() ?? "No file";
  }
}

function addLcd() {
  const first = devices.lcdDevices[0];
  config.addLcd({
    serial: first?.device_id ?? null,
    index: first ? undefined : 0,
    type: "image",
    path: null,
    fps: null,
    orientation: 0,
    rgb: null,
  });
  selectedRaw.value = entries.value.length - 1;
}
</script>

<template>
  <div class="page lcd-page">
    <MediaAccessNotice :lcds="entries" :templates="selectedTemplates" @issues="(v) => accessIssues = v">
      <template #actions>
        <n-button v-if="!accessIssues && !importActive" size="tiny" quaternary @click="copiesOpen = !copiesOpen">
          {{ copiesOpen ? "Hide copies" : "Manage copies" }}
        </n-button>
      </template>
    </MediaAccessNotice>
    <ManagedMediaImport
      v-show="showCopies"
      :lcds="entries"
      :templates="selectedTemplates"
      @in-progress="onImportProgress"
    />

    <div v-for="device in wirelessImageDevices" :key="device.device_id" class="card wireless-row">
      <span>{{ device.name }} · Wireless</span>
      <StartupImageDialog :device="device" />
    </div>

    <div class="layout">
      <aside class="card lcd-list">
        <div class="list-head">
          <span class="list-title">Screens</span>
          <n-button size="tiny" type="primary" :disabled="!devices.lcdDevices.length" @click="addLcd">
            <template #icon><Plus :size="13" /></template>
            Add
          </n-button>
        </div>

        <button
          v-for="(entry, i) in entries"
          :key="i"
          class="lcd-item"
          :class="{ active: i === selected, offline: !deviceName(entry) }"
          @click="selectedRaw = i"
        >
          <component :is="mediaMeta(entry).icon" :size="16" class="item-icon" />
          <span class="item-text">
            <span class="item-name">LCD {{ i + 1 }} · {{ mediaMeta(entry).label }}</span>
            <span class="item-sub item-device">{{ deviceName(entry) ?? "Device offline" }}</span>
            <span v-if="summary(entry)" class="item-sub">{{ summary(entry) }}</span>
          </span>
        </button>

        <p v-if="!devices.lcdDevices.length" class="list-empty muted">
          <MonitorOff :size="16" />
          No LCD devices detected.
        </p>
        <p v-else-if="!entries.length" class="list-empty muted">
          Click "Add" to configure a screen.
        </p>
      </aside>

      <LcdConfigCard
        v-if="current"
        :key="selected"
        class="detail"
        :entry="current"
        :index="selected"
      />
      <div v-else class="card detail empty muted">
        Select or add a screen to configure it.
      </div>
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
.wireless-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.layout {
  display: grid;
  grid-template-columns: 240px minmax(0, 1fr);
  gap: var(--space-4);
  align-items: start;
}
.lcd-list {
  position: sticky;
  top: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-3);
}
.list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-2);
}
.list-title {
  font-weight: 600;
}
.lcd-item {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  width: 100%;
  padding: var(--space-2);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: none;
  color: var(--text-primary);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.lcd-item:hover {
  background: var(--bg-hover);
}
.lcd-item.active {
  background: var(--accent-soft);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}
.lcd-item.active .item-icon {
  color: var(--accent);
}
.item-icon {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--text-secondary);
}
.item-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.item-name {
  font-size: var(--font-size-sm);
  font-weight: 600;
}
.item-sub {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lcd-item.offline .item-device {
  color: var(--warning);
}
.list-empty {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: var(--space-2) 0 0;
  font-size: var(--font-size-sm);
}
.empty {
  padding: var(--space-6);
  text-align: center;
}
@media (max-width: 760px) {
  .layout {
    grid-template-columns: 1fr;
  }
  .lcd-list {
    position: static;
  }
}
</style>
