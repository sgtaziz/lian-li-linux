<script setup lang="ts">
import { computed } from "vue";
import { ArrowDown, ArrowUp } from "lucide-vue-next";
import type {
  MergeLightingConfig,
  RgbDeviceCapabilities,
  RgbEffect,
  RgbEffectParameters,
  RgbSyncKind,
} from "@/types";
import { useConfigStore } from "@/stores/config";
import RgbEffectControls from "@/components/rgb/RgbEffectControls.vue";
import { copyEffect, recallSyncEffect, rememberSyncEffect } from "@/components/rgb/effectMemory";
import { continuousParametersFor, createDefaultSync, matchedParametersFor, MATCHED_MODES } from "@/components/rgb/syncSettings";
import { modeLabel } from "@/constants";

const config = useConfigStore();
const rgb = computed(() => config.ensureRgb());
const storedSync = computed(() => rgb.value.merge_lighting ?? null);
const visibleCaps = computed(() => config.rgbCaps.filter((cap) => !cap.rf_owned));

function motherboardSynced(cap: RgbDeviceCapabilities): boolean {
  return rgb.value.devices.find((device) => device.device_id === cap.device_id)?.mb_rgb_sync ?? false;
}

function defaultSync(): MergeLightingConfig {
  return createDefaultSync(visibleCaps.value.filter((cap) => !motherboardSynced(cap)));
}

const sync = computed(() => storedSync.value ?? defaultSync());

function ensureSync(): MergeLightingConfig {
  if (!rgb.value.merge_lighting) rgb.value.merge_lighting = defaultSync();
  const value = rgb.value.merge_lighting;
  while (value.directions.length < value.device_order.length) value.directions.push("Clockwise");
  return value;
}

function continuousEligible(cap: RgbDeviceCapabilities): boolean {
  return cap.sync_led_count != null;
}

function matchedEligible(cap: RgbDeviceCapabilities, mode = sync.value.effect.mode): boolean {
  return cap.supported_modes.includes(mode);
}

const candidates = computed(() => visibleCaps.value.filter((cap) =>
  sync.value.kind === "Continuous"
    ? continuousEligible(cap)
    : MATCHED_MODES.some((mode) => cap.supported_modes.includes(mode)),
));

const orderedCandidates = computed(() => {
  const positions = new Map(sync.value.device_order.map((id, index) => [id, index]));
  return [...candidates.value].sort((a, b) =>
    (positions.get(a.device_id) ?? Number.MAX_SAFE_INTEGER)
    - (positions.get(b.device_id) ?? Number.MAX_SAFE_INTEGER),
  );
});

function isEligible(cap: RgbDeviceCapabilities, mode = sync.value.effect.mode): boolean {
  if (motherboardSynced(cap)) return false;
  return sync.value.kind === "Continuous" ? continuousEligible(cap) : matchedEligible(cap, mode);
}

function isSelected(cap: RgbDeviceCapabilities): boolean {
  const included = storedSync.value
    ? sync.value.device_order.includes(cap.device_id)
    : sync.value.device_order.includes(cap.device_id);
  return included && !sync.value.disabled_devices.includes(cap.device_id);
}

function isEnabled(cap: RgbDeviceCapabilities): boolean {
  return isSelected(cap) && isEligible(cap);
}

const participantCount = computed(() => candidates.value.filter(isEnabled).length);

const enabled = computed({
  get: () => sync.value.enabled,
  set: (value: boolean) => {
    if (value && (participantCount.value < 2 || !modeOptions.value.some((option) => option.value === sync.value.effect.mode))) return;
    ensureSync().enabled = value;
    config.markDirty();
  },
});

const kind = computed({
  get: () => sync.value.kind,
  set: (value: RgbSyncKind) => changeKind(value),
});

function setDeviceEnabled(cap: RgbDeviceCapabilities, value: boolean) {
  const current = ensureSync();
  if (value) {
    if (!current.device_order.includes(cap.device_id)) {
      current.device_order.push(cap.device_id);
      current.directions.push("Clockwise");
    }
    current.disabled_devices = current.disabled_devices.filter((id) => id !== cap.device_id);
  } else if (!current.disabled_devices.includes(cap.device_id)) {
    current.disabled_devices.push(cap.device_id);
  }
  if (current.enabled && participantCount.value < 2) current.enabled = false;
  config.markDirty();
}

function moveDevice(deviceId: string, offset: -1 | 1) {
  const current = ensureSync();
  const displayed = orderedCandidates.value.filter(isSelected).map((cap) => cap.device_id);
  const displayedIndex = displayed.indexOf(deviceId);
  const otherId = displayed[displayedIndex + offset];
  if (!otherId) return;
  const index = current.device_order.indexOf(deviceId);
  const otherIndex = current.device_order.indexOf(otherId);
  [current.device_order[index], current.device_order[otherIndex]] = [current.device_order[otherIndex], current.device_order[index]];
  [current.directions[index], current.directions[otherIndex]] = [current.directions[otherIndex], current.directions[index]];
  config.markDirty();
}

function canMoveDevice(cap: RgbDeviceCapabilities, offset: -1 | 1): boolean {
  const selected = orderedCandidates.value.filter(isSelected);
  const index = selected.findIndex((candidate) => candidate.device_id === cap.device_id);
  return index >= 0 && index + offset >= 0 && index + offset < selected.length;
}

function deviceReversed(deviceId: string): boolean {
  const index = sync.value.device_order.indexOf(deviceId);
  return sync.value.directions[index] === "CounterClockwise";
}

function setDeviceReversed(deviceId: string, reversed: boolean) {
  const current = ensureSync();
  const index = current.device_order.indexOf(deviceId);
  current.directions[index] = reversed ? "CounterClockwise" : "Clockwise";
  config.markDirty();
}

function continuousParameters(): RgbEffectParameters[] {
  const selected = candidates.value.filter((cap) => isSelected(cap) && !motherboardSynced(cap));
  const participants = selected.length ? selected : candidates.value;
  return continuousParametersFor(participants);
}

function matchedParameters(mode: string): RgbEffectParameters | undefined {
  const participants = candidates.value.filter((cap) => isSelected(cap) && !motherboardSynced(cap));
  return matchedParametersFor(mode, participants);
}

function parametersFor(kindValue: RgbSyncKind, mode: string): RgbEffectParameters | undefined {
  if (kindValue === "Continuous") return continuousParameters().find((entry) => entry.mode === mode);
  return matchedParameters(mode);
}

const parameters = computed(() => parametersFor(sync.value.kind, sync.value.effect.mode));
const matchedModeOptions = computed(() => {
  const selected = candidates.value.filter((cap) => isSelected(cap) && !motherboardSynced(cap));
  return MATCHED_MODES.filter((mode) => selected.every((cap) => cap.supported_modes.includes(mode)));
});
const modeOptions = computed(() => (sync.value.kind === "Continuous"
  ? continuousParameters().map((entry) => entry.mode)
  : matchedModeOptions.value
).map((mode) => ({ label: modeLabel(mode), value: mode })));

function changeKind(value: RgbSyncKind) {
  const current = ensureSync();
  if (current.kind === value) return;
  current.effect_memory = rememberSyncEffect(current.effect_memory, current.effect);
  current.kind = value;
  ensureSync();
  const modes = value === "Continuous"
    ? continuousParameters().map((entry) => entry.mode)
    : matchedModeOptions.value;
  const mode = modes.includes(current.effect.mode) ? current.effect.mode : (modes[0] ?? current.effect.mode);
  current.effect = effectForMode(current, value, mode);
  config.markDirty();
}

function effectForMode(current: MergeLightingConfig, kindValue: RgbSyncKind, mode: string): RgbEffect {
  const remembered = recallSyncEffect(current.effect_memory, mode);
  if (remembered) return { ...remembered, mode, scope: "All" };
  const next = copyEffect(current.effect);
  const nextParameters = parametersFor(kindValue, mode);
  next.mode = mode;
  next.scope = "All";
  next.colors = next.colors.slice(0, nextParameters?.max_colors ?? 0);
  while (next.colors.length < (nextParameters?.min_colors ?? 0)) next.colors.push([255, 255, 255]);
  if (nextParameters?.directions.length && !nextParameters.directions.includes(next.direction)) {
    next.direction = nextParameters.directions[0];
  }
  return next;
}

function setMode(mode: string) {
  const current = ensureSync();
  if (current.effect.mode === mode) return;
  current.effect_memory = rememberSyncEffect(current.effect_memory, current.effect);
  current.effect = effectForMode(current, current.kind, mode);
  config.markDirty();
}

function patchEffect(patch: Partial<RgbEffect>) {
  const current = ensureSync();
  current.effect = { ...current.effect, ...patch };
  config.markDirty();
}
</script>

<template>
  <section class="card sync-card">
    <div class="sync-head">
      <div>
        <h2 class="section-title">Quick Sync</h2>
        <p class="muted sync-description">Run one coordinated effect across multiple devices.</p>
      </div>
      <n-switch v-model:value="enabled" :disabled="!enabled && (participantCount < 2 || !modeOptions.some((option) => option.value === sync.effect.mode))" />
    </div>

    <div class="sync-options">
      <div class="field">
        <label class="muted">Sync style</label>
        <n-radio-group v-model:value="kind" size="small">
          <n-radio-button value="Continuous">Continuous strip</n-radio-button>
          <n-radio-button value="Matched">Matched effect</n-radio-button>
        </n-radio-group>
      </div>
      <div class="field">
        <label class="muted">Effect</label>
        <n-select :value="sync.effect.mode" :options="modeOptions" size="small" @update:value="setMode" />
      </div>
    </div>

    <p v-if="participantCount < 2" class="hint">Choose at least two compatible devices to enable Quick Sync.</p>
    <p v-else-if="!modeOptions.some((option) => option.value === sync.effect.mode)" class="hint">Choose an effect supported by every enabled device.</p>
    <p v-if="enabled" class="sync-active">Quick Sync controls the enabled devices below. Save to apply changes.</p>

    <div class="devices">
      <div v-for="cap in orderedCandidates" :key="cap.device_id" class="sync-device">
        <n-checkbox
          :checked="isSelected(cap)"
          :disabled="!isSelected(cap) && !isEligible(cap)"
          @update:checked="(value) => setDeviceEnabled(cap, value)"
        >
          {{ cap.device_name }}
        </n-checkbox>
        <span v-if="motherboardSynced(cap)" class="muted unavailable">Motherboard sync enabled</span>
        <span v-else-if="!isEligible(cap)" class="muted unavailable">Effect unavailable</span>
        <span v-else-if="kind === 'Continuous'" class="muted led-count">{{ cap.sync_led_count }} LEDs</span>
        <n-switch
          v-if="kind === 'Continuous' && !['RainbowMorph', 'Twinkle'].includes(sync.effect.mode)"
          :value="deviceReversed(cap.device_id)"
          :disabled="!isSelected(cap)"
          size="small"
          @update:value="(value) => setDeviceReversed(cap.device_id, value)"
        >
          <template #checked>Reverse</template>
          <template #unchecked>Forward</template>
        </n-switch>
        <div class="order-buttons">
          <n-button quaternary size="tiny" :disabled="!canMoveDevice(cap, -1)" @click="moveDevice(cap.device_id, -1)">
            <template #icon><ArrowUp :size="14" /></template>
          </n-button>
          <n-button quaternary size="tiny" :disabled="!canMoveDevice(cap, 1)" @click="moveDevice(cap.device_id, 1)">
            <template #icon><ArrowDown :size="14" /></template>
          </n-button>
        </div>
      </div>
    </div>

    <RgbEffectControls v-if="parameters" :effect="sync.effect" :parameters="parameters" @update="patchEffect" />
  </section>
</template>

<style scoped>
.sync-card, .field { display: flex; flex-direction: column; gap: var(--space-3); }
.sync-head, .sync-device { display: flex; align-items: center; gap: var(--space-3); }
.sync-head { justify-content: space-between; }
.section-title, .sync-description { margin: 0; }
.sync-options { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); }
.devices { display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; }
.sync-device { min-height: 42px; padding: var(--space-2) var(--space-3); border-bottom: 1px solid var(--border); }
.sync-device:last-child { border-bottom: 0; }
.unavailable, .led-count { margin-left: auto; font-size: var(--font-size-sm); }
.order-buttons { display: flex; margin-left: var(--space-1); }
.sync-active { margin: 0; color: var(--primary-color); font-size: var(--font-size-sm); }
@media (max-width: 700px) { .sync-options { grid-template-columns: 1fr; } }
</style>
