import { defineStore } from "pinia";
import { emit } from "@tauri-apps/api/event";
import { LCD_TEMPLATES_CHANGED_EVENT } from "@/stores/lcd";
import { computed, reactive, ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { tr } from "@/i18n";
import { isSpeedModeKey } from "@/components/fans/speedMode";
import type {
  AppConfig,
  LcdTemplate,
  RgbDeviceCapabilities,
  RgbPreset,
  SensorInfo,
  LcdConfig,
  FanCurve,
  FanSpeed,
  PwmHeader,
} from "@/types";

function defaultConfig(): AppConfig {
  return {
    turn_off_lcds_on_shutdown: true,
    default_fps: 30,
    hardware_video: false,
    hid_backend: "hidraw",
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
  const savedLcds = ref<LcdConfig[]>([]);
  const dirty = ref(false);
  const loaded = ref(false);

  const rgbCaps = ref<RgbDeviceCapabilities[]>([]);
  const sensors = ref<SensorInfo[]>([]);
  const templates = ref<LcdTemplate[]>([]);
  const imported = ref<{ instance: string; originals: LcdTemplate[]; copies: LcdTemplate[] } | null>(null);
  const presets = ref<RgbPreset[]>([]);
  const pwmHeaders = ref<PwmHeader[]>([]);

  function markDirty() {
    dirty.value = true;
  }

  /** Replace the entire config object (used after a daemon reload). */
  function replace(next: AppConfig) {
    savedLcds.value = structuredClone(next.lcds);
    Object.assign(config, defaultConfig(), next);
    dirty.value = false;
    loaded.value = true;
    imported.value = null;
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

  async function refreshTemplates() {
    const current = await ipc.request<LcdTemplate[]>("GetLcdTemplates");
    const copies = imported.value?.copies ?? [];
    const ids = new Set(copies.map((item) => item.id));
    templates.value = [...current.filter((item) => !ids.has(item.id)), ...JSON.parse(JSON.stringify(copies))];
  }

  async function save() {
    // Flush any pending debounced effect requests first.
    await Promise.all([...flushRegistry].map((fn) => fn()));
    const staged = imported.value;
    if (staged?.copies.length) {
      const current = await ipc.request<LcdTemplate[]>("GetLcdTemplates", null, staged.instance);
      for (const copy of staged.copies) {
        if (JSON.stringify(current.find((item) => item.id === copy.id)) !== JSON.stringify(staged.originals.find((item) => item.id === copy.id))) {
          throw new Error(tr("shell.config.copiedTemplateChanged"));
        }
      }
      await ipc.request("MergeLcdTemplates", { originals: staged.originals, copies: staged.copies }, staged.instance);
      staged.originals = JSON.parse(JSON.stringify(staged.copies));
      await emit(LCD_TEMPLATES_CHANGED_EVENT);
    }
    try {
      await ipc.request("SetConfig", { config }, staged?.instance);
    } catch (error) {
      if (staged?.copies.length) throw new Error(tr("shell.config.lcdNotConfirmed", { error: String(error) }));
      throw error;
    }
    dirty.value = false;
    await load();
  }

  function stageImportedMedia(lcds: LcdConfig[], copies: LcdTemplate[], instance: string) {
    if (imported.value) throw new Error(tr("shell.config.previousImportPending"));
    imported.value = { instance, originals: JSON.parse(JSON.stringify(templates.value)), copies: JSON.parse(JSON.stringify(copies)) };
    const ids = new Set(copies.map((item) => item.id));
    templates.value = [...templates.value.filter((item) => !ids.has(item.id)), ...JSON.parse(JSON.stringify(copies))];
    config.lcds = JSON.parse(JSON.stringify(lcds));
    markDirty();
  }

  // Registry of flush callbacks invoked before save (debounced RGB/direction).
  const flushRegistry = new Set<() => void | Promise<void>>();
  function registerFlush(fn: () => void | Promise<void>) {
    flushRegistry.add(fn);
    return () => flushRegistry.delete(fn);
  }

  // ── Convenience accessors ──────────────────────────────────────────────────
  const rgb = computed(() => config.rgb);
  const fans = computed(() => config.fans);
  const thermalAlert = computed(() => config.thermal_alert);

  function ensureRgb() {
    if (!config.rgb) {
      config.rgb = { enabled: true, openrgb_server: false, openrgb_port: 6743, openrgb_regions: false, openrgb_per_fan_zones: false, devices: [] };
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

  async function refreshRgbCapabilities() {
    rgbCaps.value = await ipc.request<RgbDeviceCapabilities[]>("GetRgbCapabilities");
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

  function fanCurveInUse(name: string): boolean {
    if (isSpeedModeKey(name)) return false;
    const fanSpeeds = (config.fans?.speeds ?? []).flatMap((group) => group.speeds);
    const aioSpeeds = Object.values(config.aio).flatMap((aio) => [aio.pump_target_rpm, ...aio.fan_speeds]);
    return [...fanSpeeds, ...aioSpeeds].includes(name);
  }

  function renameFanCurve(curve: FanCurve, name: string) {
    const previous = curve.name;
    if (previous === name) return;
    const modeKey = isSpeedModeKey(previous);
    const follow = (speed: FanSpeed) => (!modeKey && speed === previous ? name : speed);
    for (const group of config.fans?.speeds ?? []) {
      group.speeds = group.speeds.map(follow);
    }
    for (const aio of Object.values(config.aio)) {
      aio.pump_target_rpm = follow(aio.pump_target_rpm);
      aio.fan_speeds = aio.fan_speeds.map(follow);
    }
    curve.name = name;
    markDirty();
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
    savedLcds,
    dirty,
    loaded,
    rgbCaps,
    sensors,
    templates,
    imported,
    stageImportedMedia,
    presets,
    pwmHeaders,
    rgb,
    fans,
    thermalAlert,
    markDirty,
    replace,
    load,
    refreshTemplates,
    save,
    registerFlush,
    ensureRgb,
    ensureFans,
    rgbCapsFor,
    refreshRgbCapabilities,
    rgbDeviceConfig,
    presetsFor,
    aioConfigFor,
    curveNames,
    fanCurveInUse,
    renameFanCurve,
    addLcd,
    addFanCurve,
  };
});

function defaultAio() {
  return {
    pump_target_rpm: "off" as const,
    fan_speeds: ["off", "off", "off", "off"] as string[],
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
