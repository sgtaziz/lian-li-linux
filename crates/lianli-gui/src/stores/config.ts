import { defineStore } from "pinia";
import { computed, reactive, ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import type {
  AppConfig,
  LcdTemplate,
  RgbDeviceCapabilities,
  RgbPreset,
  SensorInfo,
  LcdConfig,
  FanCurve,
  PwmHeader,
} from "@/types";

function defaultConfig(): AppConfig {
  return {
    default_fps: 30,
    lcds: [],
    fan_curves: [],
    fans: null,
    rgb: null,
    aio: {},
    ene6k77: {},
    thermal_alert: {
      cpu: { enabled: false, threshold: 80, alert_color: [255, 0, 0] },
      gpu: { enabled: false, threshold: 80, alert_color: [0, 0, 255] },
    },
    rgb_drift_detection_enabled: true,
    rgb_drift_detection_interval_ms: 1000,
  };
}

/**
 * Authoritative mirror of the daemon's AppConfig plus the auxiliary lookups
 * (RGB capabilities, sensors, templates, presets) fetched on load.
 *
 * Dirty tracking: any in-place mutation should call `markDirty()`. The header
 * Save button calls `save()`, which flushes debounced effect queues first,
 * sends SetConfig, then reloads.
 */
export const useConfigStore = defineStore("config", () => {
  const ipc = useIpc();

  const config = reactive<AppConfig>(defaultConfig());
  const dirty = ref(false);
  const loaded = ref(false);

  const rgbCaps = ref<RgbDeviceCapabilities[]>([]);
  const sensors = ref<SensorInfo[]>([]);
  const templates = ref<LcdTemplate[]>([]);
  const presets = ref<RgbPreset[]>([]);
  const pwmHeaders = ref<PwmHeader[]>([]);

  function markDirty() {
    dirty.value = true;
  }

  /** Replace the entire config object (used after a daemon reload). */
  function replace(next: AppConfig) {
    Object.assign(config, defaultConfig(), next);
    dirty.value = false;
    loaded.value = true;
  }

  async function load() {
    const [cfg, caps, sens, tpls, pres, pwm] = await Promise.all([
      ipc.request<AppConfig | null>("GetConfig").catch(() => null),
      ipc.request<RgbDeviceCapabilities[]>("GetRgbCapabilities").catch(() => []),
      ipc.request<SensorInfo[]>("ListSensors").catch(() => []),
      ipc.request<LcdTemplate[]>("GetLcdTemplates").catch(() => []),
      ipc.request<RgbPreset[]>("ListRgbPresets").catch(() => []),
      ipc.request<PwmHeader[]>("ListPwmHeaders").catch(() => []),
    ]);
    if (cfg) replace(cfg);
    rgbCaps.value = caps ?? [];
    sensors.value = sens ?? [];
    templates.value = tpls ?? [];
    presets.value = pres ?? [];
    pwmHeaders.value = pwm ?? [];
    loaded.value = true;
  }

  async function save() {
    // Flush any pending debounced effect requests first.
    flushRegistry.forEach((fn) => fn());
    await ipc.request("SetConfig", { config });
    dirty.value = false;
    await load();
  }

  // Registry of flush callbacks invoked before save (debounced RGB/direction).
  const flushRegistry = new Set<() => void>();
  function registerFlush(fn: () => void) {
    flushRegistry.add(fn);
    return () => flushRegistry.delete(fn);
  }

  // ── Convenience accessors ──────────────────────────────────────────────────
  const rgb = computed(() => config.rgb);
  const fans = computed(() => config.fans);
  const thermalAlert = computed(() => config.thermal_alert);

  function ensureRgb() {
    if (!config.rgb) {
      config.rgb = { enabled: true, openrgb_server: false, openrgb_port: 6743, devices: [] };
    }
    return config.rgb;
  }

  function ensureFans() {
    if (!config.fans) {
      config.fans = {
        speeds: [],
        update_interval_ms: 1000,
        hysteresis_temp: 1.0,
        hysteresis_pwm: 5,
      };
    }
    return config.fans;
  }

  function rgbCapsFor(deviceId: string): RgbDeviceCapabilities | undefined {
    return rgbCaps.value.find((c) => c.device_id === deviceId);
  }

  function rgbDeviceConfig(deviceId: string) {
    const rgb = ensureRgb();
    let dev = rgb.devices.find((d) => d.device_id === deviceId);
    if (!dev) {
      dev = { device_id: deviceId, mb_rgb_sync: false, zones: [] };
      rgb.devices.push(dev);
    }
    return dev;
  }

  function presetsFor(deviceId: string): RgbPreset[] {
    return presets.value.filter((p) => p.device_id === deviceId);
  }

  function aioConfigFor(deviceId: string) {
    if (!config.aio[deviceId]) {
      config.aio[deviceId] = defaultAio();
    }
    return config.aio[deviceId];
  }

  function curveNames(): string[] {
    return config.fan_curves.map((c) => c.name);
  }

  function addLcd(entry: LcdConfig) {
    config.lcds.push(entry);
    markDirty();
  }

  function addFanCurve(curve: FanCurve) {
    config.fan_curves.push(curve);
    markDirty();
  }

  return {
    config,
    dirty,
    loaded,
    rgbCaps,
    sensors,
    templates,
    presets,
    pwmHeaders,
    rgb,
    fans,
    thermalAlert,
    markDirty,
    replace,
    load,
    save,
    registerFlush,
    ensureRgb,
    ensureFans,
    rgbCapsFor,
    rgbDeviceConfig,
    presetsFor,
    aioConfigFor,
    curveNames,
    addLcd,
    addFanCurve,
  };
});

function defaultAio() {
  return {
    pump_target_rpm: 128 as const,
    fan_speeds: [128, 128, 128, 128] as number[],
    theme_index: 0,
    brightness: 80,
    rotation: 0,
    loop_interval: 3,
    cpu_temp_source: null,
    cpu_load_source: { type: "cpu_usage" as const },
    gpu_temp_source: null,
    gpu_load_source: null,
    str_color: [255, 255, 255, 255] as [number, number, number, number],
    val_color: [255, 255, 255, 255] as [number, number, number, number],
    unit_color: [255, 255, 255, 255] as [number, number, number, number],
  };
}
