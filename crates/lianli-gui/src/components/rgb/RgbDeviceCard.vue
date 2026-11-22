<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { ChevronDown, ChevronUp, Save, Trash2, Locate } from "lucide-vue-next";
import type { RgbDeviceCapabilities, RgbEffect, RgbMode, RgbPortMerge } from "@/types";
import { useRgbStore } from "@/stores/rgb";
import { useConfigStore } from "@/stores/config";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { useIpc } from "@/composables/useIpc";
import EditableName from "@/components/common/EditableName.vue";
import RgbZoneEditor from "@/components/rgb/RgbZoneEditor.vue";
import RgbPreview from "@/components/rgb/RgbPreview.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import RgbRegionEditor from "@/components/rgb/RgbRegionEditor.vue";
import { mergePortOrder, mergeSiblings, mergeSuspended, motherboardSyncEnabled, portsMerged, quickSyncControls } from "@/components/rgb/syncSettings";
import { useTranslate } from "@/i18n";
import { backendText } from "@/utils/backendText";

const props = defineProps<{ cap: RgbDeviceCapabilities; label: string }>();

const DEFAULT_MERGED_EFFECT: RgbEffect = {
  mode: "Rainbow",
  colors: [],
  speed: 2,
  brightness: 4,
  direction: "Clockwise",
  scope: "All",
  disabled: false,
};

const t = useTranslate();
const rgb = useRgbStore();
const config = useConfigStore();
const names = useDeviceNamesStore();
const dialog = useDialog();
const ipc = useIpc();

async function ping() {
  try {
    await ipc.request("PingDevice", { device_id: props.cap.device_id, zone: 0 });
  } catch (e) {
    dialog.error({
      title: t("rgb.deviceCard.pingFailed"),
      content: String(e),
      positiveText: t("common.ok"),
    });
  }
}

const devConfig = computed(() => config.rgbDeviceConfig(props.cap.device_id));

const regional = computed(() => (props.cap.effect_regions?.length ?? 0) > 0);
const portGroups = computed(() => (props.cap.group_effect_modes?.length ?? 0) > 0);
const controlTab = ref<"regions" | "direct">("regions");
const controlTabOptions = computed(() => [
  { value: "regions" as const, label: t("rgb.deviceCard.groupEffects") },
  ...(props.cap.supports_direct || portGroups.value
    ? [{ value: "direct" as const, label: t("rgb.deviceCard.directZones") }]
    : []),
]);
watch(() => devConfig.value.zones[0]?.effect.scope, (scope) => {
  if (portGroups.value) controlTab.value = scope === "Fan" ? "direct" : "regions";
}, { immediate: true });
const visibleZones = computed(() => devConfig.value.zones.slice(0, props.cap.zones.length));
const syncActive = computed(() => quickSyncControls(props.cap, config.ensureRgb()));

watch(
  [devConfig, () => props.cap.zones.length],
  ([cfg]) => {
    const cap = props.cap;
    if (cap.hardware_group_effects) return;
    const mode = cfg.zones.length > 0 && cfg.zones.every((zone) => zone.effect.mode === "Direct") ? "Direct" : "Static";
    while (cfg.zones.length < cap.zones.length) {
      cfg.zones.push({
        zone_index: cfg.zones.length,
        effect: {
          mode,
          colors: [[255, 255, 255]],
          speed: 2,
          brightness: 4,
          direction: "Clockwise",
          scope: "All",
          disabled: false,
        },
        swap_lr: false,
        swap_tb: false,
      });
    }
  },
  { immediate: true },
);

const mbSync = computed({
  get: () => motherboardSyncEnabled(props.cap, config.ensureRgb()),
  set: (v: boolean) => {
    // MB sync is controller-wide — propagate to all sibling ports.
    const baseId = props.cap.device_id.replace(/:(?:port|group)\d+$/, "");
    const rgbCfg = config.ensureRgb();
    for (const dev of rgbCfg.devices) {
      if (dev.device_id.replace(/:(?:port|group)\d+$/, "") === baseId) {
        dev.mb_rgb_sync = v;
        dev.active_preset = null;
        if (v && !props.cap.hardware_group_effects) {
          for (const zone of dev.zones) {
            zone.effect = { ...zone.effect, mode: "Static" };
          }
        }
      }
    }
    // One IPC call — hardware applies controller-wide.
    void rgb.setMbSync(props.cap.device_id, v);
    config.markDirty();
  },
});

const mergeIds = computed(() => mergeSiblings(props.cap, config.rgbCaps));
const canMerge = computed(() => mergeIds.value.length > 1);
const mergeLeader = computed(() => mergeIds.value[0]);
const leadMerge = computed(() => config.rgbDeviceConfig(mergeLeader.value).port_merge);

function newPortMerge(): RgbPortMerge {
  const effect = (): RgbEffect => ({ ...DEFAULT_MERGED_EFFECT });
  return { enabled: false, effect: effect(), split_sides: false, sync_sides: true, top: effect(), bottom: effect(), port_order: [] };
}

/** Every sibling carries the same copy, so the daemon finds it whichever port connects. */
function syncMerge() {
  const source = leadMerge.value;
  for (const id of mergeIds.value) {
    const dev = config.rgbDeviceConfig(id);
    if (source && id !== mergeLeader.value) dev.port_merge = JSON.parse(JSON.stringify(source));
    dev.active_preset = null;
  }
  config.markDirty();
}

function editMerge(change: (merge: RgbPortMerge) => void) {
  const lead = config.rgbDeviceConfig(mergeLeader.value);
  lead.port_merge ??= newPortMerge();
  change(lead.port_merge);
  syncMerge();
}

const merged = computed({
  get: () => portsMerged(props.cap, config.ensureRgb()),
  set: (v: boolean) => editMerge((merge) => (merge.enabled = v)),
});
const splitSides = computed({
  get: () => leadMerge.value?.split_sides ?? false,
  set: (v: boolean) => editMerge((merge) => (merge.split_sides = v)),
});
const syncSides = computed({
  get: () => leadMerge.value?.sync_sides ?? true,
  set: (v: boolean) => editMerge((merge) => (merge.sync_sides = v)),
});
const reversePort = computed({
  get: () => devConfig.value.reverse_port ?? false,
  set: (v: boolean) => {
    devConfig.value.reverse_port = v;
    config.markDirty();
  },
});

const mergePorts = computed(() => mergeIds.value.map((id) => Number(id.match(/:port(\d+)$/)?.[1] ?? 0)));
const portOrder = computed(() => mergePortOrder(leadMerge.value?.port_order, mergePorts.value));
function movePort(index: number, offset: number) {
  const order = [...portOrder.value];
  const target = index + offset;
  if (target < 0 || target >= order.length) return;
  [order[index], order[target]] = [order[target], order[index]];
  editMerge((merge) => (merge.port_order = order));
}
function portName(port: number) {
  const id = mergeIds.value[mergePorts.value.indexOf(port)];
  return names.nameOf(id) || config.rgbCaps.find((cap) => cap.device_id === id)?.device_name || id;
}

/** Static merges as one colour per side. */
function mergedCapFor(modes: RgbMode[] | undefined): RgbDeviceCapabilities {
  const params = (props.cap.effect_parameters ?? []).map((entry) =>
    entry.mode === "Static" ? { ...entry, min_colors: 1, max_colors: 2, per_fan_colors: false } : entry,
  );
  return { ...props.cap, group_effect_modes: modes ?? [], effect_parameters: params, supported_scopes: [] };
}
const mergeFollower = computed(() => merged.value && mergeLeader.value !== props.cap.device_id);
const mergePaused = computed(() => mergeSuspended(props.cap, config.rgbCaps, config.ensureRgb()));
const wholeCap = computed(() => mergedCapFor(props.cap.merged_effect_modes));
const sideCap = computed(() => mergedCapFor(props.cap.merged_side_effect_modes));
const mergeZone = (key: "effect" | "top" | "bottom") =>
  computed(() => leadMerge.value && { zone_index: 0, effect: leadMerge.value[key], swap_lr: false, swap_tb: false });
const wholeZone = mergeZone("effect");
const topZone = mergeZone("top");
const bottomZone = mergeZone("bottom");
const leaderName = computed(() => portName(mergePorts.value[0]));
const mergeLayoutOptions = computed(() => [
  { value: "whole", label: t("rgb.deviceCard.mergeWholeFan") },
  { value: "sides", label: t("rgb.deviceCard.mergeSides") },
]);

function zoneAt(i: number) {
  return devConfig.value.zones[i];
}

function applyToAllZones() {
  const src = devConfig.value.zones[0];
  if (!src) return;
  for (let i = 1; i < devConfig.value.zones.length; i++) {
    devConfig.value.zones[i].effect = JSON.parse(JSON.stringify(src.effect));
  }
  devConfig.value.active_preset = null;
  config.markDirty();
}

// ── Presets ─────────────────────────────────────────────────────────────────
const presetName = ref("");
const presetOptions = computed(() =>
  config.presetsFor(props.cap.device_id).map((p) => ({ label: p.name, value: p.name })),
);

async function savePreset() {
  if (!presetName.value.trim()) return;
  await rgb.savePreset(presetName.value.trim(), props.cap.device_id);
  presetName.value = "";
}
async function applyPreset(name: string) {
  await rgb.applyPreset(name, props.cap.device_id);
  // The daemon saved the preset into this device's config; other unsaved edits stay.
  await config.load({ keepDraft: true, rgbDevice: props.cap.device_id });
}
async function deletePreset(name: string) {
  dialog.error({
    title: t("rgb.deviceCard.deletePresetTitle"),
    content: t("rgb.deviceCard.deletePresetContent", { name }),
    positiveText: t("common.delete"),
    negativeText: t("common.cancel"),
    onPositiveClick: () => void rgb.deletePreset(name, props.cap.device_id),
  });
}

const activePreset = computed({
  get: () => devConfig.value.active_preset ?? null,
  set: (v) => {
    devConfig.value.active_preset = v;
    config.markDirty();
  },
});

const totalLedCount = computed(() => {
  const control = props.cap.fan_led_count_control;
  if (!control) return props.cap.total_led_count;
  const reported = props.cap.zones[control.zone]?.led_count ?? control.default;
  return props.cap.total_led_count - reported + (devConfig.value.fan_led_count ?? control.default);
});
const summary = computed(() =>
  t("rgb.deviceCard.summary", { zones: props.cap.zones.length, leds: totalLedCount.value }),
);
</script>

<template>
  <div class="card rgb-device">
    <div class="head">
      <div class="title-wrap">
        <EditableName
          :name="names.nameOf(cap.device_id)"
          :label="label"
          @rename="(name) => names.setName(cap.device_id, name)"
        />
        <span class="device-label">{{ summary }}</span>
      </div>
      <button class="ping-btn" :title="t('rgb.deviceCard.identifyDevice')" @click="ping">
        <Locate :size="14" />
      </button>
    </div>

    <p v-if="cap.deferred_reason" class="notice warning">{{ backendText(cap.deferred_reason, cap.deferred_reason_ui) }}</p>
    <p v-if="syncActive" class="notice">{{ t(cap.fan_led_count_control ? "rgb.deviceCard.quickSyncNoticeLedCount" : "rgb.deviceCard.quickSyncNotice") }}</p>

    <RgbPreview v-if="!syncActive && !merged" :cap="cap" :config="devConfig" :mb-sync="mbSync" />

    <div class="device-controls">
      <div v-if="cap.supports_mb_rgb_sync || canMerge || cap.supports_port_reverse || (!mbSync && (regional || portGroups))" class="toolbar">
        <div v-if="!mbSync && !merged && (regional || portGroups)" class="control-tabs" :inert="syncActive || undefined">
          <SegmentedControl
            :model-value="controlTab"
            :options="controlTabOptions"
            :label="t('rgb.deviceCard.groupEffects')"
            @update:model-value="(tab) => (controlTab = tab)"
          />
        </div>
        <div class="toggles" :class="{ 'sync-locked': syncActive }" :inert="syncActive || undefined">
          <n-checkbox v-if="canMerge && !mbSync" v-model:checked="merged">{{ t("rgb.deviceCard.mergePorts") }}</n-checkbox>
          <n-checkbox v-if="cap.supports_port_reverse && !mbSync" v-model:checked="reversePort">{{ t("rgb.deviceCard.reversePort") }}</n-checkbox>
          <n-checkbox v-if="cap.supports_mb_rgb_sync" v-model:checked="mbSync">{{ t("rgb.deviceCard.motherboardSync") }}</n-checkbox>
        </div>
      </div>

      <p v-if="mbSync" class="hint">{{ t("rgb.deviceCard.motherboardControls") }}</p>
      <p v-else-if="mergePaused" class="hint">{{ t("rgb.deviceCard.mergeSuspended") }}</p>
      <p v-else-if="mergeFollower" class="hint">{{ t("rgb.deviceCard.mergedFollower", { port: leaderName }) }}</p>
      <template v-else-if="merged && leadMerge">
        <div class="merge-settings" :class="{ 'sync-locked': syncActive }" :inert="syncActive || undefined">
          <SegmentedControl
            :model-value="splitSides ? 'sides' : 'whole'"
            :options="mergeLayoutOptions"
            :label="t('rgb.deviceCard.mergeLayout')"
            @update:model-value="(layout) => (splitSides = layout === 'sides')"
          />
          <n-checkbox v-if="splitSides" v-model:checked="syncSides">{{ t("rgb.deviceCard.syncSides") }}</n-checkbox>
        </div>
        <RgbZoneEditor v-if="!splitSides && wholeZone" :device-id="mergeLeader" :cap="wholeCap" :zone-index="0" :zone="wholeZone" :lighting-locked="syncActive" group-effects merged-ports @change="syncMerge" />
        <template v-else>
          <RgbZoneEditor v-if="topZone" :device-id="mergeLeader" :cap="sideCap" :zone-index="0" :zone="topZone" :title="t(syncSides ? 'rgb.zoneEditor.bothSides' : 'rgb.zoneEditor.topSide')" :lighting-locked="syncActive" group-effects merged-ports @change="syncMerge" />
          <RgbZoneEditor v-if="!syncSides && bottomZone" :device-id="mergeLeader" :cap="sideCap" :zone-index="0" :zone="bottomZone" :title="t('rgb.zoneEditor.bottomSide')" :lighting-locked="syncActive" group-effects merged-ports @change="syncMerge" />
        </template>
        <div class="port-order" :class="{ 'sync-locked': syncActive }" :inert="syncActive || undefined">
          <span class="muted">{{ t("rgb.deviceCard.portOrder") }}</span>
          <ol>
            <li v-for="(port, index) in portOrder" :key="port">
              <span>{{ portName(port) }}</span>
              <n-button size="tiny" quaternary :disabled="index === 0" :title="t('rgb.deviceCard.moveUp')" :aria-label="t('rgb.deviceCard.moveUp')" @click="movePort(index, -1)">
                <template #icon><ChevronUp :size="14" /></template>
              </n-button>
              <n-button size="tiny" quaternary :disabled="index === portOrder.length - 1" :title="t('rgb.deviceCard.moveDown')" :aria-label="t('rgb.deviceCard.moveDown')" @click="movePort(index, 1)">
                <template #icon><ChevronDown :size="14" /></template>
              </n-button>
            </li>
          </ol>
        </div>
      </template>
      <template v-else>
        <p v-if="cap.hardware_group_effects" class="hint">{{ t("rgb.deviceCard.hardwareGroupHint") }}</p>
        <p v-if="cap.hardware_group_effects && !devConfig.regions" class="hint">{{ t("rgb.deviceCard.hardwareGroupFanHint") }}</p>
        <RgbRegionEditor v-if="regional && controlTab === 'regions'" :device-id="cap.device_id" :cap="cap" :inert="syncActive || undefined" :class="{ 'sync-locked': syncActive }" />
        <RgbZoneEditor v-else-if="portGroups && controlTab === 'regions' && visibleZones[0]" :device-id="cap.device_id" :cap="cap" :zone-index="0" :zone="visibleZones[0]" :lighting-locked="syncActive" group-effects />
        <div v-if="(!regional && !portGroups) || controlTab === 'direct'" class="zones">
          <RgbZoneEditor
            v-for="(z, i) in visibleZones"
            :key="i"
            :device-id="cap.device_id"
            :cap="cap"
            :zone-index="i"
            :zone="z"
            :direct-only="regional && controlTab === 'direct'"
            :per-fan-only="portGroups && controlTab === 'direct'"
            :lighting-locked="syncActive"
          />
        </div>

        <div class="preset-bar" :class="{ 'sync-locked': syncActive }" :inert="syncActive || undefined">
          <n-button v-if="!regional && !portGroups" size="small" @click="applyToAllZones">{{ t("rgb.deviceCard.applyToAllZones") }}</n-button>
          <span class="preset-label muted">{{ t("rgb.deviceCard.preset") }}</span>
          <n-select
            :value="activePreset"
            size="small"
            :options="presetOptions"
            :placeholder="t('rgb.deviceCard.noPreset')"
            @update:value="applyPreset"
            style="width: 180px"
          />
          <n-button
            v-if="activePreset"
            size="small"
            @click="activePreset && applyPreset(activePreset)"
          >
            {{ t("common.apply") }}
          </n-button>
          <n-button
            size="small"
            quaternary
            type="error"
            :title="t('rgb.deviceCard.deletePreset')"
            :disabled="!activePreset"
            @click="activePreset && deletePreset(activePreset)"
          >
            <template #icon><Trash2 :size="14" /></template>
          </n-button>
          <n-input v-model:value="presetName" size="small" :placeholder="t('rgb.deviceCard.presetName')" style="width: 140px" />
          <n-button size="small" @click="savePreset">
            <template #icon><Save :size="14" /></template>
            {{ t("rgb.deviceCard.saveAs") }}
          </n-button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.rgb-device {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.title-wrap {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  font-family: var(--font-display);
  font-size: var(--font-size-xl);
  font-weight: 600;
}
.device-label {
  font-family: var(--font-sans);
  font-size: 13px;
  font-weight: 400;
  color: var(--md-on-surface-variant);
  white-space: nowrap;
}
.ping-btn {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border: none;
  border-radius: 50%;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
  cursor: pointer;
  transition: background 0.15s;
}
.ping-btn:hover {
  background: var(--md-surface-container-highest);
}
.ping-btn:focus-visible {
  outline: 2px solid var(--md-primary);
}
.notice {
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border-radius: 14px;
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
  font-size: var(--font-size-sm);
}
.notice.warning {
  background: color-mix(in srgb, var(--warning) 16%, var(--md-surface-container));
  color: var(--md-on-surface);
}
.hint {
  margin: 0;
}
.device-controls {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.sync-locked {
  opacity: 0.5;
  pointer-events: none;
}
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
}
.control-tabs {
  display: flex;
  gap: var(--space-2);
}
.toggles {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-3);
  margin-left: auto;
}
.merge-settings {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-4);
}
.port-order {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.port-order ol {
  margin: 0;
  padding-left: var(--space-5, 20px);
}
.port-order li > span {
  display: inline-block;
  min-width: 160px;
}
.zones {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.preset-bar {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  flex-wrap: wrap;
  border-top: 1px solid var(--md-outline-variant);
  padding-top: var(--space-3);
}
.preset-label {
  font-size: var(--font-size-sm);
}
.preset-bar > .n-button:first-child {
  margin-right: auto;
}
</style>
