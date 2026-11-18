<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { Save, Trash2, Locate } from "lucide-vue-next";
import type { RgbDeviceCapabilities } from "@/types";
import { useRgbStore } from "@/stores/rgb";
import { useConfigStore } from "@/stores/config";
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { useIpc } from "@/composables/useIpc";
import EditableName from "@/components/common/EditableName.vue";
import RgbZoneEditor from "@/components/rgb/RgbZoneEditor.vue";
import RgbPreview from "@/components/rgb/RgbPreview.vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import RgbRegionEditor from "@/components/rgb/RgbRegionEditor.vue";
import { motherboardSyncEnabled, quickSyncControls } from "@/components/rgb/syncSettings";
import { useTranslate } from "@/i18n";
import { backendText } from "@/utils/backendText";

const props = defineProps<{ cap: RgbDeviceCapabilities; label: string }>();

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
  // Reload config so the dropdown reflects the newly-active preset
  // (the daemon persisted active_preset + the preset's effects).
  await config.load();
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

    <RgbPreview v-if="!syncActive" :cap="cap" :config="devConfig" :mb-sync="mbSync" />

    <div class="device-controls">
      <div v-if="cap.supports_mb_rgb_sync || (!mbSync && (regional || portGroups))" class="toolbar">
        <div v-if="!mbSync && (regional || portGroups)" class="control-tabs" :inert="syncActive || undefined">
          <SegmentedControl
            :model-value="controlTab"
            :options="controlTabOptions"
            :label="t('rgb.deviceCard.groupEffects')"
            @update:model-value="(tab) => (controlTab = tab)"
          />
        </div>
        <n-checkbox
          v-if="cap.supports_mb_rgb_sync"
          v-model:checked="mbSync"
          class="mb-sync"
          :class="{ 'sync-locked': syncActive }"
          :inert="syncActive || undefined"
        >{{ t("rgb.deviceCard.motherboardSync") }}</n-checkbox>
      </div>

      <p v-if="mbSync" class="hint">{{ t("rgb.deviceCard.motherboardControls") }}</p>
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
.mb-sync {
  margin-left: auto;
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
