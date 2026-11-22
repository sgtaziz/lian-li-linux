<script setup lang="ts">
import { computed, ref } from "vue";
import { Layers, Palette, Wifi } from "lucide-vue-next";
import type { RgbDeviceCapabilities } from "@/types";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { useDevicesStore } from "@/stores/devices";
import { rgbDeviceLabels } from "@/utils/deviceNames";
import RgbDeviceCard from "@/components/rgb/RgbDeviceCard.vue";
import RgbSyncEditor from "@/components/rgb/RgbSyncEditor.vue";
import OpenRgbOptions from "@/components/rgb/OpenRgbOptions.vue";
import StatusDot from "@/components/common/StatusDot.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import { mergeSuspended, motherboardSyncEnabled, portsMerged, quickSyncControls } from "@/components/rgb/syncSettings";
import { useTranslate } from "@/i18n";
import { backendText } from "@/utils/backendText";

const QUICK_SYNC = "quick-sync";

const t = useTranslate();
const daemon = useDaemonStore();
const config = useConfigStore();
const names = useDeviceNamesStore();
const devices = useDevicesStore();

const rgb = computed(() => config.ensureRgb());

const openrgbEnabled = computed({
  get: () => rgb.value.openrgb_server,
  set: (v: boolean) => {
    rgb.value.openrgb_server = v;
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
  if (!openrgbEnabled.value) return t("common.disabled");
  if (daemon.openrgbError) return t("rgb.page.openrgbError", { error: backendText(daemon.openrgbError, daemon.openrgbErrorUi) });
  if (daemon.openrgbRunning) return t("rgb.page.openrgbListening", { port: daemon.openrgbPort ?? rgb.value.openrgb_port });
  return t("rgb.page.openrgbStarting");
});

const rgbCaps = computed(() => config.rgbCaps.filter((c) => !c.rf_owned));
const labels = computed(() => rgbDeviceLabels(rgbCaps.value, devices.labels));
function defaultLabel(cap: RgbDeviceCapabilities): string {
  return labels.value.get(cap.device_id) ?? cap.device_name;
}

const syncedCount = computed(() => rgbCaps.value.filter((cap) => quickSyncControls(cap, rgb.value)).length);
const syncSummary = computed(() => {
  if (!rgb.value.merge_lighting?.enabled) return t("common.off");
  return t(syncedCount.value === 1 ? "rgb.page.syncOnOne" : "rgb.page.syncOnMany", { count: syncedCount.value });
});

const selectedRaw = ref<string | null>(null);
const selected = computed(() => {
  const wanted = selectedRaw.value;
  if (!rgbCaps.value.length) return null;
  if (wanted === QUICK_SYNC || (wanted && rgbCaps.value.some((cap) => cap.device_id === wanted))) return wanted;
  if (rgb.value.merge_lighting?.enabled) return QUICK_SYNC;
  return rgbCaps.value[0]?.device_id ?? null;
});
const selectedCap = computed(() => rgbCaps.value.find((cap) => cap.device_id === selected.value));

type Tone = "success" | "warning" | "muted" | "info";
function capState(cap: RgbDeviceCapabilities): { label: string; tone: Tone } {
  if (!daemon.connected) return { label: t("common.offline"), tone: "muted" };
  if (cap.deferred_reason) return { label: t("rgb.page.stateWaiting"), tone: "warning" };
  if (openrgbEnabled.value) return { label: "OpenRGB", tone: "info" };
  if (motherboardSyncEnabled(cap, rgb.value)) return { label: t("rgb.page.stateMotherboard"), tone: "info" };
  if (portsMerged(cap, rgb.value) && !mergeSuspended(cap, rgbCaps.value, rgb.value)) return { label: t("rgb.page.stateMerged"), tone: "info" };
  if (quickSyncControls(cap, rgb.value)) return { label: t("rgb.quickSync"), tone: "info" };
  return { label: t("common.online"), tone: "success" };
}

function isWireless(cap: RgbDeviceCapabilities): boolean {
  return cap.device_id.startsWith("wireless:");
}

function capSummary(cap: RgbDeviceCapabilities): string {
  const count = cap.zones.length;
  if (isWireless(cap)) return t(count === 1 ? "rgb.page.wirelessZoneCountOne" : "rgb.page.wirelessZoneCountMany", { count });
  return t(count === 1 ? "rgb.page.zoneCountOne" : "rgb.page.zoneCountMany", { count });
}
</script>

<template>
  <div class="page rgb-page">
    <section class="card openrgb">
      <div class="openrgb-row">
        <span class="openrgb-title">OpenRGB</span>
        <span class="status-tag">
          <StatusDot
            :color="openrgbEnabled ? (daemon.openrgbRunning ? 'success' : 'warning') : 'muted'"
          />
          {{ statusText }}
        </span>
        <div class="openrgb-controls">
          <label v-if="openrgbEnabled" class="port-row">
            <span class="muted">{{ t("rgb.page.port") }}</span>
            <n-input-number
              v-model:value="openrgbPort"
              size="small"
              :min="1"
              :max="65535"
            />
          </label>
          <n-checkbox v-model:checked="openrgbEnabled">{{ t("rgb.page.enableOpenrgb") }}</n-checkbox>
        </div>
      </div>
      <template v-if="openrgbEnabled">
        <p class="hint">{{ t("rgb.page.openrgbWarning") }}</p>
        <OpenRgbOptions />
      </template>
    </section>

    <div class="layout">
      <aside class="card rgb-list">
        <div class="list-head">
          <span class="list-title">{{ t("rgb.page.lighting") }}</span>
          <span v-if="rgbCaps.length" class="list-count muted">{{ t(rgbCaps.length === 1 ? "rgb.page.deviceCountOne" : "rgb.page.deviceCountMany", { count: rgbCaps.length }) }}</span>
        </div>

        <template v-if="rgbCaps.length">
          <div
            class="rgb-item"
            :class="{ active: selected === QUICK_SYNC }"
            role="button"
            tabindex="0"
            @click="selectedRaw = QUICK_SYNC"
            @keydown.enter="selectedRaw = QUICK_SYNC"
          >
            <span class="item-icon sync"><Layers :size="18" /></span>
            <span class="item-text">
              <span class="item-name">{{ t("rgb.quickSync") }}</span>
              <span class="item-sub">{{ syncSummary }}</span>
            </span>
          </div>
          <div class="list-divider" />
        </template>

        <div
          v-for="cap in rgbCaps"
          :key="cap.device_id"
          class="rgb-item"
          :class="{ active: cap.device_id === selected, offline: !daemon.connected }"
          role="button"
          tabindex="0"
          @click="selectedRaw = cap.device_id"
          @keydown.enter="selectedRaw = cap.device_id"
        >
          <span class="item-icon"><component :is="isWireless(cap) ? Wifi : Palette" :size="18" /></span>
          <span class="item-text">
            <span class="item-name">{{ names.label(cap.device_id, defaultLabel(cap)) }}</span>
            <span class="item-sub">
              <span class="state" :class="`tone-${capState(cap).tone}`" :title="capState(cap).label">
                <span class="dot" />
                <template v-if="capState(cap).tone !== 'success'">{{ capState(cap).label }}</template>
              </span>
              {{ capSummary(cap) }}
            </span>
          </span>
        </div>

        <p v-if="!rgbCaps.length" class="list-empty muted">
          <Palette :size="16" />
          {{ t("rgb.page.noRgbDevices") }}
        </p>
      </aside>

      <RgbSyncEditor v-if="selected === QUICK_SYNC" class="detail" />
      <RgbDeviceCard
        v-else-if="selectedCap"
        :key="selectedCap.device_id"
        class="detail"
        :cap="selectedCap"
        :label="defaultLabel(selectedCap)"
      />
      <EmptyState v-else class="detail" kind="rgb" />
    </div>
  </div>
</template>

<style scoped>
.rgb-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.openrgb {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: 14px 20px;
  border-radius: 22px;
}
.openrgb-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-4);
}
.openrgb-title {
  font-size: 15px;
  font-weight: 600;
}
.status-tag {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 3px 10px;
  border-radius: 10px;
  background: var(--md-surface-container-highest);
  font-size: var(--font-size-xs);
  font-weight: 600;
  color: var(--md-on-surface-variant);
  white-space: nowrap;
}
.openrgb-controls {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin-left: auto;
}
.port-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
}
.port-row .n-input-number {
  width: 110px;
}
.openrgb .hint {
  margin: 0;
}
.layout {
  display: grid;
  grid-template-columns: 280px minmax(0, 1fr);
  gap: var(--space-4);
  align-items: start;
}
.rgb-list {
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
.list-count {
  font-size: var(--font-size-xs);
}
.list-divider {
  height: 1px;
  margin: var(--space-1) var(--space-2);
  background: var(--md-outline-variant);
}
.rgb-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  box-sizing: border-box;
  padding: 10px var(--space-3) 10px var(--space-2);
  border-radius: 18px;
  color: var(--md-on-surface);
  cursor: pointer;
  transition: background 0.15s;
}
.rgb-item:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
}
.rgb-item:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -2px;
}
.rgb-item.active {
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
  background: var(--md-tertiary-container);
  color: var(--md-on-tertiary-container);
}
.item-icon.sync {
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
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--font-size-xs);
  color: var(--md-on-surface-variant);
  overflow: hidden;
  white-space: nowrap;
}
.rgb-item.active .item-sub {
  color: inherit;
  opacity: 0.8;
}
.rgb-item.offline .item-name {
  color: var(--md-outline);
}
.state {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.tone-success { color: var(--success); }
.tone-warning { color: var(--warning); }
.tone-info { color: var(--accent); }
.tone-muted { color: var(--text-muted); }
.list-empty {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: var(--space-2) 0 0;
  font-size: var(--font-size-sm);
}
@media (max-width: 1000px) {
  .layout {
    grid-template-columns: 1fr;
  }
  .rgb-list {
    position: static;
  }
}
</style>
