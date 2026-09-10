<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Plus, Trash2 } from "lucide-vue-next";
import type { RgbDeviceCapabilities, RgbEffect, RgbRegionConfig, RgbScope, RGB, RGBA } from "@/types";
import { useConfigStore } from "@/stores/config";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import LabeledSlider from "@/components/common/LabeledSlider.vue";
import { recallDeviceEffect, rememberDeviceEffect } from "@/components/rgb/effectMemory";
import { RGB_BRIGHTNESS, RGB_DIRECTIONS, modeLabel } from "@/constants";

const props = defineProps<{ deviceId: string; cap: RgbDeviceCapabilities }>();
const config = useConfigStore();

const devConfig = computed(() => config.rgbDeviceConfig(props.deviceId));
const scopes = computed(() => props.cap.region_parameters?.map((r) => r.scope) ?? ["All", ...(props.cap.effect_regions ?? []).filter((scope) => scope !== "All")] as RgbScope[]);
const regions = computed(() => devConfig.value.regions ?? []);
const selected = ref<RgbScope>(regions.value.some((r) => regionScope(r) === "All") ? "All" : (regions.value[0]?.effect.scope ?? scopes.value[0] ?? "All"));
watch(scopes, (available) => {
  if (!available.includes(selected.value)) selected.value = available[0] ?? "All";
}, { immediate: true });
const currentIndex = computed(() => {
  const exact = regions.value.findIndex((r) => regionScope(r) === selected.value);
  if (exact >= 0) return exact;
  const all = regions.value.findIndex((r) => regionScope(r) === "All");
  return all;
});
const current = computed(() => regions.value[currentIndex.value] ?? previewRegion(selected.value));
const mixedSelection = computed(() => selected.value === "All" && currentIndex.value < 0 && regions.value.length > 0);
const scopedParameters = computed(() => props.cap.region_parameters?.find((r) => r.scope === selected.value)?.effects ?? props.cap.effect_parameters);
const parameters = computed(() => mixedSelection.value ? undefined : scopedParameters.value?.find((p) => p.mode === current.value?.effect.mode));
const isPalette = computed(() => (parameters.value?.max_colors ?? 0) > 0);
const perFanColors = computed(() => parameters.value?.per_fan_colors ?? false);
const maxColors = computed(() => parameters.value?.max_colors ?? 0);
const minColors = computed(() => parameters.value?.min_colors ?? 0);
const paletteColors = computed(() => {
  const colors = current.value?.effect.colors ?? [];
  const screen = props.cap.render_profile?.family === "UniversalScreen" || props.cap.render_profile?.family === "HydroShiftIIOled";
  const count = perFanColors.value ? maxColors.value : screen ? Math.max(minColors.value, colors.length) : colors.length;
  const fallback: RGB = perFanColors.value ? [0, 0, 0] : colors[colors.length - 1] ?? [0, 0, 0];
  return Array.from({ length: count }, (_, index) => colors[index] ?? fallback);
});
const directionOptions = computed(() => RGB_DIRECTIONS.filter((d) => parameters.value?.directions.includes(d.value)));
const copyToSegments = computed(() => props.cap.render_profile?.family === "Strimer" && selected.value !== "All");
const canApplyAll = computed(() => !!current.value && (copyToSegments.value
  ? scopes.value.filter((scope) => scope !== "All").every((scope) => supportsMode(scope, current.value!.effect.mode))
  : supportsMode("All", current.value.effect.mode)));
const modeOptions = computed(() =>
  (scopedParameters.value?.map((p) => p.mode) ?? (props.cap.software_modes?.length
    ? props.cap.software_modes
    : props.cap.supported_modes.filter((mode) => mode !== "Direct")))
    .filter((mode) => mode !== "Direct")
    .map((mode) => ({ label: modeLabel(mode), value: mode })),
);

function regionScope(region: RgbRegionConfig): RgbScope {
  return region.effect.scope;
}

function supportsMode(scope: RgbScope, mode: string): boolean {
  const options = props.cap.region_parameters?.find((r) => r.scope === scope)?.effects;
  return options ? options.some((option) => option.mode === mode) : true;
}

function previewRegion(scope: RgbScope): RgbRegionConfig | undefined {
  if (!regions.value.length) return undefined;
  const family = props.cap.render_profile?.family;
  if (family === "Strimer") {
    return { effect: { ...defaultEffect(scope), mode: "Rainbow", colors: [], speed: 3 }, flip: false };
  }
  const inheritedScopes: RgbScope[] = family === "Sl" || family === "SlV4" ? ["Inner", "Outer"]
    : family === "SlInf" || family === "SlInfV3" ? ["Outer", "Inner", "Center"] : [];
  const inherited = inheritedScopes.map((side) => regions.value.find((region) => regionScope(region) === side)).find(Boolean);
  if (inherited) return { effect: { ...inherited.effect, scope }, flip: inherited.flip };
  return { effect: { ...defaultEffect(scope), mode: "Off", colors: [] }, flip: regions.value[0].flip };
}

function defaultEffect(scope: RgbScope): RgbEffect {
  const options = props.cap.region_parameters?.find((r) => r.scope === scope)?.effects;
  const effect = options?.find((p) => p.mode === "Static") ?? options?.find((p) => p.mode !== "Off") ?? options?.[0];
  const colors = effect?.max_colors ?? 1;
  return { mode: effect?.mode ?? "Static", colors: Array.from({ length: colors }, () => [255, 255, 255] as RGB), speed: 2, brightness: 4, direction: effect?.directions[0] ?? "Clockwise", scope, disabled: false };
}

function startRegions() {
  const scope = scopes.value[0] ?? "All";
  devConfig.value.regions = [{ effect: defaultEffect(scope), flip: false }];
  devConfig.value.active_preset = null;
  selected.value = scope;
  config.markDirty();
}

function ensureExpanded(scope: RgbScope) {
  if (!devConfig.value.regions?.length) return;
  if (scope !== "All" && devConfig.value.regions.length === 1 && regionScope(devConfig.value.regions[0]) === "All") {
    const all = devConfig.value.regions[0];
    devConfig.value.regions = (props.cap.effect_regions ?? []).filter((region) => region !== "All")
      .filter((region) => supportsMode(region, all.effect.mode))
      .map((region) => ({ effect: { ...all.effect, colors: all.effect.colors.map((color) => [...color] as RGB), scope: region }, flip: all.flip }));
  }
}

function selectScope(scope: RgbScope) {
  selected.value = scope;
}

function touchRegion() {
  const list = devConfig.value.regions;
  if (!list || currentIndex.value < 0) return;
  const [last] = list.splice(currentIndex.value, 1);
  list.push(last);
  config.rgbDeviceConfig(props.deviceId).active_preset = null;
  config.markDirty();
}

function patchEffect(patch: Partial<RgbEffect>, flip?: boolean) {
  if (!current.value) return;
  const seed = current.value;
  ensureExpanded(selected.value);
  const index = regions.value.findIndex((r) => regionScope(r) === selected.value);
  if (index < 0 && selected.value === "All") {
    devConfig.value.regions = [{ effect: { ...seed.effect, scope: "All" }, flip: seed.flip }];
  } else if (index < 0) {
    devConfig.value.regions?.push({ effect: { ...seed.effect, colors: seed.effect.colors.map((color) => [...color] as RGB), scope: selected.value }, flip: seed.flip });
  }
  const target = regions.value[regions.value.findIndex((r) => regionScope(r) === selected.value)];
  if (!target) return;
  target.effect = { ...target.effect, ...patch };
  if (flip !== undefined) target.flip = flip;
  touchRegion();
}
function onColor(index: number, value: RGB | RGBA) {
  const colors = [...paletteColors.value];
  colors[index] = [value[0], value[1], value[2]];
  patchEffect({ colors });
}
function addColor() { patchEffect({ colors: [...(current.value?.effect.colors ?? []), [255, 255, 255]] }); }
function removeColor(index: number) {
  if ((current.value?.effect.colors.length ?? 0) <= minColors.value) return;
  patchEffect({ colors: current.value!.effect.colors.filter((_, i) => i !== index) });
}
function onMode(mode: string) {
  if (!current.value || current.value.effect.mode === mode) return;
  rememberDeviceEffect(devConfig.value, null, current.value.effect, current.value.flip);
  const remembered = recallDeviceEffect(devConfig.value, null, selected.value, mode);
  if (remembered) {
    patchEffect({ ...remembered.effect, scope: selected.value }, remembered.flip);
    return;
  }
  const options = scopedParameters.value?.find((p) => p.mode === mode);
  const count = Math.min(options?.max_colors ?? 0, Math.max(options?.min_colors ?? 0, current.value?.effect.colors.length ?? 0));
  const colors = [...(current.value?.effect.colors ?? [])].slice(0, count);
  while (colors.length < count) colors.push([255, 255, 255]);
  const direction = current.value?.effect.direction ?? "Clockwise";
  patchEffect({ mode, colors, scope: selected.value, direction: options?.directions.length && !options.directions.includes(direction) ? options.directions[0] : direction });
}

function applyToAll() {
  if (!current.value || !devConfig.value.regions || !canApplyAll.value) return;
  const effect = current.value.effect;
  devConfig.value.regions = (copyToSegments.value ? scopes.value.filter((scope) => scope !== "All") : ["All" as RgbScope])
    .map((scope) => ({ effect: { ...effect, colors: effect.colors.map((color) => [...color] as RGB), scope }, flip: current.value!.flip }));
  if (!copyToSegments.value) selected.value = "All";
  devConfig.value.active_preset = null;
  config.markDirty();
}

function flipGroup(flip: boolean) {
  for (const region of devConfig.value.regions ?? []) region.flip = flip;
  devConfig.value.active_preset = null;
  config.markDirty();
}
</script>

<template>
  <div class="regional">
    <div v-if="!devConfig.regions?.length" class="migration card-inset">
      <span>Animations apply to device regions. Choose group regions to replace the existing per-zone settings.</span>
      <n-button size="small" @click="startRegions">Use group regions</n-button>
    </div>
    <template v-else>
      <div class="region-tabs">
        <n-button v-for="scope in scopes" :key="scope" size="small" :type="selected === scope ? 'primary' : 'default'" @click="selectScope(scope)">{{ scope }}</n-button>
      </div>
      <div v-if="cap.render_profile?.family === 'Tl' && current" class="row">
        <label class="muted">Swap top and bottom</label>
        <n-switch :value="current.flip" @update:value="flipGroup" />
      </div>
      <n-button v-if="selected !== 'All' && current && scopes.includes('All')" :disabled="!canApplyAll" size="small" quaternary @click="applyToAll">{{ copyToSegments ? 'Apply to all segments' : 'Apply this region to All' }}</n-button>
      <div v-if="current" class="region-body card-inset">
        <div class="row"><label class="muted">Mode</label><n-select size="small" :value="mixedSelection ? null : current.effect.mode" placeholder="Mixed region effects" :options="modeOptions" @update:value="onMode" /></div>
        <div v-if="isPalette" class="colors">
          <label class="muted">Colors</label>
          <div class="color-list">
            <div v-for="(_, i) in paletteColors" :key="i" class="color-item">
              <span v-if="perFanColors" class="muted">Fan {{ i + 1 }}</span>
              <ColorPicker class="region-color" :model-value="paletteColors[i]" @update:model-value="(v) => onColor(i, v)" />
              <n-button v-if="!perFanColors && current.effect.colors.length > minColors" quaternary size="small" type="error" @click="removeColor(i)"><Trash2 :size="14" /></n-button>
            </div>
            <n-button v-if="!perFanColors && current.effect.colors.length < maxColors" size="small" quaternary @click="addColor"><Plus :size="14" /></n-button>
          </div>
        </div>
        <div v-if="parameters" class="two-col">
          <LabeledSlider v-if="parameters?.supports_speed" label="Speed" :model-value="current.effect.speed" :min="0" :max="4" @update:model-value="(v) => patchEffect({ speed: v })" />
          <div class="field"><label class="muted">Brightness</label><n-select size="small" :value="current.effect.brightness" :options="RGB_BRIGHTNESS" @update:value="(v) => patchEffect({ brightness: v })" /></div>
        </div>
        <div v-if="directionOptions.length" class="field"><label class="muted">Direction</label><n-select size="small" :value="current.effect.direction" :options="directionOptions" @update:value="(v) => patchEffect({ direction: v })" /></div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.regional, .region-body { display: flex; flex-direction: column; gap: var(--space-3); }
.migration { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
.region-tabs, .row, .color-list { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
.row { justify-content: space-between; }
.colors, .field { display: flex; flex-direction: column; gap: var(--space-1); }
.color-item { display: flex; flex-shrink: 0; align-items: center; gap: var(--space-1); }
.region-color { min-width: 130px; }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); }
</style>
