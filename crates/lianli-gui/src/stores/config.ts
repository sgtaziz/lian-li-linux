import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, LcdConfig, FanConfig, FanCurve, RgbAppConfig } from "../types";

export const useConfigStore = defineStore("config", () => {
  const config = ref<AppConfig | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const dirty = ref(false);

  const lcds = computed(() => config.value?.lcds ?? []);
  const fanCurves = computed(() => config.value?.fan_curves ?? []);
  const fanConfig = computed(() => config.value?.fans ?? null);
  const rgbConfig = computed(() => config.value?.rgb ?? null);

  async function load() {
    try {
      loading.value = true;
      config.value = await invoke<AppConfig>("get_config");
      dirty.value = false;
      error.value = null;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function save() {
    if (!config.value) return;
    try {
      loading.value = true;
      await invoke("set_config", { config: config.value });
      dirty.value = false;
      error.value = null;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function updateLcd(index: number, lcd: LcdConfig) {
    if (!config.value) return;
    config.value.lcds[index] = lcd;
    dirty.value = true;
  }

  function addLcd(lcd: LcdConfig) {
    if (!config.value) return;
    config.value.lcds.push(lcd);
    dirty.value = true;
  }

  function removeLcd(index: number) {
    if (!config.value) return;
    config.value.lcds.splice(index, 1);
    dirty.value = true;
  }

  function updateFanCurves(curves: FanCurve[]) {
    if (!config.value) return;
    config.value.fan_curves = curves;
    dirty.value = true;
  }

  function updateFanConfig(fans: FanConfig) {
    if (!config.value) return;
    config.value.fans = fans;
    dirty.value = true;
  }

  function setDefaultFps(fps: number) {
    if (!config.value) return;
    config.value.default_fps = fps;
    dirty.value = true;
  }

  function updateRgbConfig(rgb: RgbAppConfig) {
    if (!config.value) return;
    config.value.rgb = rgb;
    dirty.value = true;
  }

  return {
    config,
    loading,
    error,
    dirty,
    lcds,
    fanCurves,
    fanConfig,
    rgbConfig,
    load,
    save,
    updateLcd,
    addLcd,
    removeLcd,
    updateFanCurves,
    updateFanConfig,
    setDefaultFps,
    updateRgbConfig,
  };
});
