<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDialog } from "naive-ui";
import { Plus, Trash2 } from "lucide-vue-next";
import type { RgbDeviceCapabilities, RgbZoneConfig, RgbEffect, RGB } from "@/types";
import { useRgbStore } from "@/stores/rgb";
import { useConfigStore } from "@/stores/config";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import LedStrip from "@/components/rgb/LedStrip.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { RGB_DIRECTIONS, RGB_SCOPES, RGB_BRIGHTNESS, modeLabel } from "@/constants";

const props = defineProps<{
  deviceId: string;
  cap: RgbDeviceCapabilities;
  zoneIndex: number;
  zone: RgbZoneConfig;
}>();

const rgb = useRgbStore();
const config = useConfigStore();
const dialog = useDialog();

const expanded = ref(true);

const effect = computed(() => props.zone.effect);

// Direct-mode LED state (wireless + Direct mode).
const isDirect = computed(() => effect.value.mode === "Direct");
const isWireless = computed(() => props.deviceId.startsWith("wireless:"));
const ledCount = computed(() => props.cap.zones[props.zoneIndex]?.led_count ?? 0);
const ledColors = ref<RGB[]>([]);
const selectedLeds = ref<number[]>([]);

async function loadLedColors() {
  if (isDirect.value && isWireless.value && ledCount.value > 0) {
    ledColors.value = await rgb.getZoneColors(props.deviceId, props.zoneIndex);
    if (ledColors.value.length === 0) {
      ledColors.value = Array.from({ length: ledCount.value }, () => [0, 0, 0] as RGB);
    }
  }
}

watch(
  () => [isDirect.value, isWireless.value, ledCount.value, props.zoneIndex] as const,
  () => void loadLedColors(),
  { immediate: true },
);

const supportedScopes = computed(
  () => props.cap.supported_scopes?.[props.zoneIndex] ?? [],
);
const showScope = computed(() => supportedScopes.value.length > 0);
const showDirection = computed(() => props.cap.supports_direction);

// ── Effect mutation helpers ──────────────────────────────────────────────────
// RGB effects are NOT applied live — they only update the config mirror and
// take effect when the user saves (SetConfig re-applies all RGB on the daemon).

const propagateChoice = ref<"ask" | "yes" | "no">("ask");

function propagateToZones() {
  const devCfg = config.rgbDeviceConfig(props.deviceId);
  for (let i = 1; i < devCfg.zones.length; i++) {
    devCfg.zones[i].effect = JSON.parse(JSON.stringify(effect.value));
  }
}

function patchEffect(p: Partial<RgbEffect>) {
  Object.assign(effect.value, p);

  if (props.zoneIndex === 0) {
    const isPerFan =
      (effect.value.mode === "Off" ||
        effect.value.mode === "Static" ||
        effect.value.mode === "Direct") &&
      effect.value.scope === "All";
    if (!isPerFan) {
      if (propagateChoice.value === "yes") {
        propagateToZones();
      } else if (propagateChoice.value === "ask") {
        dialog.warning({
          title: "Apply to all zones?",
          content: "Animated effects on zone 0 propagate to all zones on this device.",
          positiveText: "Apply to all",
          negativeText: "Zone 0 only",
          onPositiveClick: () => {
            propagateChoice.value = "yes";
            propagateToZones();
          },
          onNegativeClick: () => {
            propagateChoice.value = "no";
          },
        });
      }
    }
  }

  // Clear any active preset — the user is manually overriding the effect, so
  // the daemon must not re-push stale preset LED colors on save.
  config.rgbDeviceConfig(props.deviceId).active_preset = null;
  config.markDirty();
}

function modeOptions() {
  return props.cap.supported_modes.map((m) => ({ label: modeLabel(m), value: m }));
}

function onMode(value: string) {
  patchEffect({ mode: value });
}

function onColor(index: number, value: RGB | any) {
  const next = [...effect.value.colors];
  next[index] = value as RGB;
  patchEffect({ colors: next });
}

function addColor() {
  patchEffect({ colors: [...effect.value.colors, [255, 255, 255]] });
}
function removeColor(index: number) {
  if (effect.value.colors.length <= 1) return;
  dialog.error({
    title: "Remove color?",
    content: `Color ${index + 1} will be removed from the palette.`,
    positiveText: "Remove",
    negativeText: "Cancel",
    onPositiveClick: () => {
      const next = effect.value.colors.filter((_, i) => i !== index);
      patchEffect({ colors: next });
    },
  });
}

function onDirection(value: any) {
  patchEffect({ direction: value });
}
function onScope(value: any) {
  patchEffect({ scope: value });
}
function onSpeed(value: number) {
  patchEffect({ speed: value });
}
function onBrightness(value: number) {
  patchEffect({ brightness: value });
}function onSwapLr(v: boolean) {
  props.zone.swap_lr = v;
  config.markDirty();
}
function onSwapTb(v: boolean) {
  props.zone.swap_tb = v;
  config.markDirty();
}

// ── Direct LED controls ──────────────────────────────────────────────────────
function onSelectLed(i: number) {
  const idx = selectedLeds.value.indexOf(i);
  if (idx >= 0) selectedLeds.value.splice(idx, 1);
  else selectedLeds.value.push(i);
}

const directColor = ref<RGB>([255, 255, 255]);
function setDirectColor(value: RGB | any) {
  directColor.value = value as RGB;
}

function applyDirect() {
  if (selectedLeds.value.length === 0) return;
  for (const i of selectedLeds.value) {
    ledColors.value[i] = directColor.value;
  }
  config.rgbDeviceConfig(props.deviceId).active_preset = null;
  config.markDirty();
  void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
}

function fillAll() {
  ledColors.value = Array.from({ length: ledCount.value }, () => directColor.value);
  config.rgbDeviceConfig(props.deviceId).active_preset = null;
  config.markDirty();
  void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
}

function clearAll() {
  dialog.error({
    title: "Clear all LEDs?",
    content: "All direct LED colors on this zone will be set to black.",
    positiveText: "Clear",
    negativeText: "Cancel",
    onPositiveClick: () => {
      ledColors.value = Array.from({ length: ledCount.value }, () => [0, 0, 0]);
      config.rgbDeviceConfig(props.deviceId).active_preset = null;
      config.markDirty();
      void rgb.sendDirect(props.deviceId, props.zoneIndex, ledColors.value);
    },
  });
}

const zoneLabel = computed(
  () => props.cap.zones[props.zoneIndex]?.name ?? `Zone ${props.zoneIndex}`,
);
</script>

<template>
  <div class="zone" :class="{ collapsed: !expanded }">
    <div class="zone-head" @click="expanded = !expanded">
      <span class="zone-name">{{ zoneLabel }}</span>
      <span class="muted">{{ ledCount }} LED(s)</span>
    </div>

    <div v-if="expanded" class="zone-body">
      <div class="row">
        <label class="muted">Mode</label>
        <n-select
          size="small"
          :value="effect.mode"
          :options="modeOptions()"
          @update:value="onMode"
          style="width: 200px"
        />
      </div>

      <!-- Direct LED editor -->
      <div v-if="isDirect && isWireless && ledCount > 0" class="direct">
        <LedStrip
          :colors="ledColors"
          :selected="selectedLeds"
          :count="ledCount"
          @select="onSelectLed"
        />
        <div class="direct-controls">
          <ColorPicker class="direct-color" :model-value="directColor" label="Color" @update:model-value="setDirectColor" />
          <n-button size="small" :disabled="!selectedLeds.length" @click="applyDirect">Apply</n-button>
          <n-button size="small" @click="fillAll">Fill All</n-button>
          <n-button size="small" @click="clearAll">Clear</n-button>
        </div>
      </div>

      <!-- Colors (hidden in Direct mode — per-LED colors are set via the LED strip above) -->
      <div v-if="!isDirect" class="colors">
        <label class="muted">Colors</label>
        <div class="color-list">
          <div v-for="(_, i) in effect.colors" :key="i" class="color-item">
            <ColorPicker
              class="zone-color"
              :model-value="effect.colors[i]"
              @update:model-value="(v) => onColor(i, v)"
            />
            <n-button quaternary size="small" type="error" @click="removeColor(i)">
              <template #icon><Trash2 :size="14" /></template>
            </n-button>
          </div>
          <n-button size="small" quaternary @click="addColor" v-if="effect.colors.length < 4">
            <template #icon><Plus :size="14" /></template>
          </n-button>
        </div>
      </div>

      <div class="two-col">
        <LabeledSlider
          label="Speed"
          :model-value="effect.speed"
          :min="0"
          :max="4"
          @update:model-value="onSpeed"
        />
        <div class="field">
          <label class="muted">Brightness</label>
          <n-select
            size="small"
            :value="effect.brightness"
            :options="RGB_BRIGHTNESS"
            @update:value="onBrightness"
          />
        </div>
      </div>

      <div v-if="showDirection" class="two-col">
        <div class="field">
          <label class="muted">Direction</label>
          <n-select
            size="small"
            :value="effect.direction"
            :options="RGB_DIRECTIONS"
            @update:value="onDirection"
          />
        </div>
        <div v-if="showScope" class="field">
          <label class="muted">Scope</label>
          <n-select
            size="small"
            :value="effect.scope"
            :options="supportedScopes.map((s) => ({ label: s, value: s }))"
            @update:value="onScope"
          />
        </div>
      </div>

      <div v-if="showDirection" class="checkboxes">
        <n-checkbox :checked="zone.swap_lr" @update:checked="onSwapLr">Swap L/R</n-checkbox>
        <n-checkbox :checked="zone.swap_tb" @update:checked="onSwapTb">Swap T/B</n-checkbox>
      </div>
    </div>
  </div>
</template>

<style scoped>
.zone {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.zone-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-2) var(--space-3);
  background: var(--bg-elevated);
  cursor: pointer;
}
.zone-name {
  font-weight: 500;
}
.zone-body {
  padding: var(--space-3);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.direct {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.direct-controls {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.direct-controls .direct-color {
  min-width: 160px;
}
.colors {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.color-list {
  display: flex;
  gap: var(--space-2);
  align-items: flex-end;
  flex-wrap: wrap;
}
.color-item {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}
/* Give the picker a definite width so its color swatch renders (in a flex row
   it otherwise collapses, showing only the hex text — same bug as gauge ranges). */
.color-item .zone-color {
  min-width: 130px;
}
.two-col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-3);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.checkboxes {
  display: flex;
  gap: var(--space-4);
}
</style>
