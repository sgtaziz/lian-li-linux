<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { Plus, Image, Film, Palette, Gauge, GripVertical, LayoutTemplate, MonitorOff } from "lucide-vue-next";
import type { LcdConfig, MediaType } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdScreensStore } from "@/stores/lcdScreens";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { lcdEntryKey, orderedLcdIndices, resolveLcdDevice } from "@/utils/lcdSelection";
import { useTranslate, type MessageKey } from "@/i18n";
import LcdConfigCard from "@/components/lcd/LcdConfigCard.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import MediaAccessNotice from "@/components/lcd/MediaAccessNotice.vue";
import ManagedMediaImport from "@/components/lcd/ManagedMediaImport.vue";
import StartupImageDialog from "@/components/lcd/StartupImageDialog.vue";

const config = useConfigStore();
const devices = useDevicesStore();
const screens = useLcdScreensStore();
const names = useDeviceNamesStore();
const t = useTranslate();

const entries = computed(() => config.config.lcds);
const ordered = computed(() => orderedLcdIndices(entries.value, screens.order));
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

const selectedRaw = ref(ordered.value[0] ?? 0);
const selected = computed(() => Math.min(selectedRaw.value, Math.max(0, entries.value.length - 1)));
const current = computed(() => entries.value[selected.value]);

const MEDIA_META: Record<MediaType, { label: MessageKey; icon: typeof Image }> = {
  image: { label: "lcd.mediaTypes.image", icon: Image },
  video: { label: "lcd.mediaTypes.video", icon: Film },
  gif: { label: "lcd.mediaTypes.gif", icon: Film },
  color: { label: "lcd.mediaTypes.color", icon: Palette },
  sensor: { label: "lcd.mediaTypes.sensor", icon: Gauge },
  custom: { label: "lcd.mediaTypes.custom", icon: LayoutTemplate },
  doublegauge: { label: "lcd.mediaTypes.legacy", icon: Gauge },
  cooler: { label: "lcd.mediaTypes.legacy", icon: Gauge },
};

function mediaMeta(entry: LcdConfig) {
  const meta = MEDIA_META[entry.type];
  return meta ? { label: t(meta.label), icon: meta.icon } : { label: entry.type, icon: Image };
}

const labels = computed(() => devices.labels);
function deviceLabel(entry: LcdConfig): string | null {
  const device = resolveLcdDevice(entry, devices.lcdDevices);
  if (!device) return null;
  return names.lcdEntryName(entry) || (labels.value.get(device.device_id) ?? device.name);
}

function moveRow(from: number, to: number) {
  if (from === to || to < 0 || to >= ordered.value.length) return;
  const next = [...ordered.value];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  screens.setOrder(next.map((index) => lcdEntryKey(entries.value[index])));
}

async function onHandleKey(position: number, step: number) {
  const to = position + step;
  if (to < 0 || to >= ordered.value.length) return;
  moveRow(position, to);
  await nextTick();
  rowElements[to]?.querySelector<HTMLElement>(".drag-handle")?.focus();
}

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
  if (from !== null && to !== null) moveRow(from, to);
}

const addOptions = computed(() => {
  const configured = new Set(
    entries.value.map((entry) => resolveLcdDevice(entry, devices.lcdDevices)?.device_id).filter(Boolean),
  );
  return devices.lcdDevices
    .filter((device) => !configured.has(device.device_id))
    .map((device) => ({ label: names.label(device.device_id, labels.value.get(device.device_id) ?? device.name), key: device.device_id }))
    .sort((a, b) => a.label.localeCompare(b.label));
});

function summary(entry: LcdConfig): string {
  switch (entry.type) {
    case "sensor":
      return entry.sensor ? `${entry.sensor.label} ${entry.sensor.unit}`.trim() : "";
    case "custom":
      return config.templates.find((t) => t.id === entry.template_id)?.name ?? t("lcd.page.noTemplate");
    case "color": {
      const [r, g, b] = entry.rgb ?? [0, 0, 0];
      return `rgb(${r}, ${g}, ${b})`;
    }
    default:
      return entry.path?.split("/").pop() ?? t("lcd.page.noFile");
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
          {{ copiesOpen ? t("lcd.page.hideCopies") : t("lcd.page.manageCopies") }}
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
      <span>{{ t("lcd.page.wirelessDevice", { name: names.label(device.device_id, device.name) }) }}</span>
      <StartupImageDialog :device="device" />
    </div>

    <div class="layout">
      <aside class="card lcd-list">
        <div class="list-head">
          <span class="list-title">{{ t("lcd.page.screens") }}</span>
          <n-dropdown trigger="click" :options="addOptions" :disabled="!addOptions.length" @select="(key: string | number) => addLcd(String(key))">
            <n-button
              size="tiny"
              type="primary"
              :disabled="!addOptions.length"
              :title="devices.lcdDevices.length && !addOptions.length ? t('lcd.page.allConfigured') : undefined"
            >
              <template #icon><Plus :size="13" /></template>
              {{ t("common.add") }}
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
            role="button"
            tabindex="0"
            :aria-label="t('lcd.page.reorder')"
            :title="t('lcd.page.reorderHint')"
            @keydown.up.prevent.stop="onHandleKey(position, -1)"
            @keydown.down.prevent.stop="onHandleKey(position, 1)"
            @keydown.enter.stop
            @pointerdown.stop="onDragStart(position, $event)"
            @pointermove="onDragMove"
            @pointerup="onDragEnd"
            @pointercancel="onDragEnd"
            @click.stop
          >
            <GripVertical :size="14" />
          </span>
          <span class="item-icon" :class="`media-${entries[index].type}`">
            <component :is="mediaMeta(entries[index]).icon" :size="18" />
          </span>
          <span class="item-text">
            <span class="item-name">{{ deviceLabel(entries[index]) ?? t("lcd.screenOffline") }}</span>
            <span class="item-sub">{{ mediaMeta(entries[index]).label }}<template v-if="summary(entries[index])"> · {{ summary(entries[index]) }}</template></span>
          </span>
        </div>

        <p v-if="!devices.lcdDevices.length" class="list-empty muted">
          <MonitorOff :size="16" />
          {{ t("lcd.page.noDevices") }}
        </p>
        <p v-else-if="!entries.length" class="list-empty muted">
          {{ t("lcd.page.addHint") }}
        </p>
      </aside>

      <LcdConfigCard
        v-if="current"
        :key="selected"
        class="detail"
        :entry="current"
        :index="selected"
      />
      <EmptyState v-else-if="!devices.lcdDevices.length" class="detail" kind="lcd" />
      <div v-else class="card detail empty muted">
        {{ t("lcd.page.selectHint") }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.lcd-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.wireless-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.layout {
  display: grid;
  grid-template-columns: 280px minmax(0, 1fr);
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
  padding: var(--space-1) var(--space-1) var(--space-2) var(--space-2);
}
.list-title {
  font-size: var(--font-size-sm);
  font-weight: 700;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}
.lcd-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  box-sizing: border-box;
  padding: 10px var(--space-3) 10px var(--space-2);
  border-radius: 18px;
  background: none;
  color: var(--md-on-surface);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.15s;
}
.lcd-item:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
}
.lcd-item:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -2px;
}
.lcd-item.active {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.item-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  border-radius: 14px;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
}
.item-icon.media-image,
.item-icon.media-gif,
.item-icon.media-video {
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.item-icon.media-sensor,
.item-icon.media-doublegauge,
.item-icon.media-cooler {
  background: var(--md-tertiary-container);
  color: var(--md-on-tertiary-container);
}
.item-icon.media-custom {
  background: var(--md-primary);
  color: var(--md-on-primary);
}
.item-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.item-name {
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.item-sub {
  font-size: var(--font-size-xs);
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lcd-item.active .item-sub {
  color: inherit;
  opacity: 0.8;
}
.lcd-item.offline .item-name {
  color: var(--warning);
}
.lcd-item.dragging {
  opacity: 0.5;
}
.lcd-item.drop-before {
  box-shadow: inset 0 3px 0 var(--md-primary);
}
.lcd-item.drop-after {
  box-shadow: inset 0 -3px 0 var(--md-primary);
}
.drag-handle {
  flex-shrink: 0;
  display: inline-flex;
  margin: 0 -6px 0 0;
  padding: 0 2px;
  color: var(--md-outline);
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
@media (max-width: 1000px) {
  .layout {
    grid-template-columns: 1fr;
  }
  .lcd-list {
    position: static;
  }
}
</style>
