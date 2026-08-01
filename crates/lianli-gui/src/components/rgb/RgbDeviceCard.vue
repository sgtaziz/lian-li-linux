<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { ChevronDown, ChevronRight, Save, Trash2, Locate } from "lucide-vue-next";
import type { RgbDeviceCapabilities } from "@/types";
import { useRgbStore } from "@/stores/rgb";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useIpc } from "@/composables/useIpc";
import RgbZoneEditor from "@/components/rgb/RgbZoneEditor.vue";

const props = defineProps<{ cap: RgbDeviceCapabilities }>();

const rgb = useRgbStore();
const config = useConfigStore();
const devices = useDevicesStore();
const dialog = useDialog();
const ipc = useIpc();

async function ping() {
  try {
    await ipc.request("PingDevice", { device_id: props.cap.device_id, zone: 0 });
  } catch (e) {
    dialog.error({
      title: "Ping failed",
      content: String(e),
      positiveText: "OK",
    });
  }
}

const devConfig = computed(() => config.rgbDeviceConfig(props.cap.device_id));
const device = computed(() => devices.byId(props.cap.device_id));

const expanded = ref(true);

// Ensure zones match the capability list.
watch(
  devConfig,
  (cfg) => {
    const cap = props.cap;
    while (cfg.zones.length < cap.zones.length) {
      cfg.zones.push({
        zone_index: cfg.zones.length,
        effect: {
          mode: "Static",
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
  get: () => devConfig.value.mb_rgb_sync,
  set: (v: boolean) => {
    // MB sync is controller-wide — propagate to all sibling ports.
    const baseId = props.cap.device_id.split(":port")[0];
    const rgbCfg = config.ensureRgb();
    for (const dev of rgbCfg.devices) {
      if (dev.device_id.split(":port")[0] === baseId) {
        dev.mb_rgb_sync = v;
        dev.active_preset = null;
        if (v) {
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
    title: "Delete preset?",
    content: `"${name}" will be permanently deleted.`,
    positiveText: "Delete",
    negativeText: "Cancel",
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

const summary = computed(() =>
  `${props.cap.zones.length} zone(s) \u00B7 ${props.cap.total_led_count} LEDs`,
);
</script>

<template>
  <div class="card rgb-device">
    <div class="head" @click="expanded = !expanded">
      <component :is="expanded ? ChevronDown : ChevronRight" :size="16" />
      <div class="head-info">
        <span class="name">{{ cap.device_name }}</span>
        <span class="muted">{{ summary }}</span>
      </div>
      <button class="ping-btn" title="Identify device" @click.stop="ping">
        <Locate :size="14" />
      </button>
    </div>

    <div v-if="expanded" class="body">
      <div v-if="cap.supports_mb_rgb_sync" class="row">
        <n-checkbox v-model:checked="mbSync">Motherboard ARGB Sync</n-checkbox>
      </div>

      <!-- When MB sync is active, hide zone config — the motherboard controls RGB -->
      <template v-if="!mbSync">
        <div class="zones">
          <RgbZoneEditor
            v-for="(z, i) in devConfig.zones"
            :key="i"
            :device-id="cap.device_id"
            :cap="cap"
            :zone-index="i"
            :zone="z"
          />
        </div>

        <div class="actions">
          <n-button size="small" @click="applyToAllZones">Apply to All Zones</n-button>
        </div>

        <!-- Preset bar -->
        <div class="preset-bar">
          <n-select
            :value="activePreset"
            size="small"
            :options="presetOptions"
            placeholder="(none)"
            @update:value="applyPreset"
            style="width: 180px"
          />
          <n-input v-model:value="presetName" size="small" placeholder="Preset name" style="width: 140px" />
          <n-button size="small" @click="savePreset">
            <template #icon><Save :size="14" /></template>
            Save As
          </n-button>
          <n-button
            size="small"
            quaternary
            type="error"
            :disabled="!activePreset"
            @click="activePreset && deletePreset(activePreset)"
          >
            <template #icon><Trash2 :size="14" /></template>
        </n-button>
        <n-button
          v-if="activePreset"
          size="small"
          @click="activePreset && applyPreset(activePreset)"
        >
          Apply
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
  gap: var(--space-3);
}
.head {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
}
.head-info {
  display: flex;
  flex-direction: column;
}
.name {
  font-weight: 600;
}
.ping-btn {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.ping-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}
.body {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.row {
  display: flex;
}
.card-inset {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--space-3);
}
.zones {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.actions {
  display: flex;
}
.preset-bar {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  flex-wrap: wrap;
  border-top: 1px solid var(--border);
  padding-top: var(--space-3);
}
</style>
