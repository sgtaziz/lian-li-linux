<script setup lang="ts">
import { computed, ref } from "vue";
import { Plus, Image, Film, Palette, Gauge, GripVertical, LayoutTemplate, MonitorOff } from "lucide-vue-next";
import type { LcdConfig, MediaType } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdNamesStore } from "@/stores/lcdNames";
import {
  lcdDeviceLabels,
  lcdEntryKey,
  loadLcdOrder,
  orderedLcdIndices,
  resolveLcdDevice,
  saveLcdOrder,
} from "@/utils/lcdSelection";
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

const lcdNames = useLcdNamesStore();
const labels = computed(() => lcdDeviceLabels(devices.lcdDevices));
function deviceLabel(entry: LcdConfig): string | null {
  const device = resolveLcdDevice(entry, devices.lcdDevices);
  if (!device) return null;
  return lcdNames.names[lcdEntryKey(entry)] || (labels.value.get(device.device_id) ?? device.name);
}

const order = ref(loadLcdOrder());
const ordered = computed(() => orderedLcdIndices(entries.value, order.value));

const rowElements: (HTMLElement | null)[] = [];
function setRowRef(element: HTMLElement | null, position: number) {
  rowElements[position] = element;
}
const dragFrom = ref<number | null>(null);
const dropAt = ref<number | null>(null);
function onDragStart(position: number, event: PointerEvent) {
  dragFrom.value = position;
  dropAt.value = position;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}
function onDragMove(event: PointerEvent) {
  if (dragFrom.value === null) return;
  const count = ordered.value.length;
  let target = count - 1;
  for (let position = 0; position < count; position++) {
    const rect = rowElements[position]?.getBoundingClientRect();
    if (rect && event.clientY < rect.top + rect.height / 2) {
      target = position;
      break;
    }
  }
  dropAt.value = target;
}
function onDragEnd() {
  const from = dragFrom.value;
  const to = dropAt.value;
  dragFrom.value = null;
  dropAt.value = null;
  if (from === null || to === null || from === to) return;
  const next = [...ordered.value];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  order.value = next.map((index) => lcdEntryKey(entries.value[index]));
  saveLcdOrder(order.value);
}

const addOptions = computed(() => {
  const configured = new Set(
    entries.value.map((entry) => resolveLcdDevice(entry, devices.lcdDevices)?.device_id).filter(Boolean),
  );
  return devices.lcdDevices
    .filter((device) => !configured.has(device.device_id))
    .map((device) => ({ label: labels.value.get(device.device_id) ?? device.name, key: device.device_id }))
    .sort((a, b) => a.label.localeCompare(b.label));
});

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

function addLcd(deviceId: string) {
  config.addLcd({
    serial: deviceId,
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
          <n-dropdown trigger="click" :options="addOptions" :disabled="!addOptions.length" @select="(key: string | number) => addLcd(String(key))">
            <n-button
              size="tiny"
              type="primary"
              :disabled="!addOptions.length"
              :title="devices.lcdDevices.length && !addOptions.length ? 'Every connected screen is already configured' : undefined"
            >
              <template #icon><Plus :size="13" /></template>
              Add
            </n-button>
          </n-dropdown>
        </div>

        <div
          v-for="(index, position) in ordered"
          :key="lcdEntryKey(entries[index]) + ':' + index"
          :ref="(element) => setRowRef(element as HTMLElement | null, position)"
          class="lcd-item"
          :class="{
            active: index === selected,
            offline: !deviceLabel(entries[index]),
            dragging: dragFrom === position,
            'drop-before': dropAt === position && dragFrom !== null && dropAt < dragFrom,
            'drop-after': dropAt === position && dragFrom !== null && dropAt > dragFrom,
          }"
          role="button"
          tabindex="0"
          @click="selectedRaw = index"
          @keydown.enter="selectedRaw = index"
        >
          <span
            v-if="ordered.length > 1"
            class="drag-handle"
            title="Drag to match the order in your case"
            @pointerdown.stop="onDragStart(position, $event)"
            @pointermove="onDragMove"
            @pointerup="onDragEnd"
            @pointercancel="onDragEnd"
            @click.stop
          >
            <GripVertical :size="14" />
          </span>
          <component :is="mediaMeta(entries[index]).icon" :size="16" class="item-icon" />
          <span class="item-text">
            <span class="item-name">{{ deviceLabel(entries[index]) ?? "Screen offline" }}</span>
            <span class="item-sub">{{ mediaMeta(entries[index]).label }}<template v-if="summary(entries[index])"> · {{ summary(entries[index]) }}</template></span>
          </span>
        </div>

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
.lcd-item.offline .item-name {
  color: var(--warning);
}
.lcd-item.dragging {
  opacity: 0.5;
}
.lcd-item.drop-before {
  box-shadow: inset 0 2px 0 var(--accent);
}
.lcd-item.drop-after {
  box-shadow: inset 0 -2px 0 var(--accent);
}
.drag-handle {
  flex-shrink: 0;
  display: inline-flex;
  margin: 1px -4px 0 -4px;
  padding: 0 2px;
  color: var(--text-muted);
  cursor: grab;
  touch-action: none;
}
.drag-handle:active {
  cursor: grabbing;
}
.lcd-item:hover .drag-handle {
  color: var(--text-secondary);
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
