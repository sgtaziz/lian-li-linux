import type {
  MergeLightingConfig,
  RgbAppConfig,
  RgbDeviceCapabilities,
  RgbEffectParameters,
} from "@/types";

export const MATCHED_MODES = ["Rainbow", "RainbowMorph", "Static", "Breathing", "Meteor", "Runway"];

export function motherboardSyncEnabled(cap: RgbDeviceCapabilities, rgb: RgbAppConfig): boolean {
  if (!cap.hardware_group_effects) {
    return rgb.devices.find((device) => device.device_id === cap.device_id)?.mb_rgb_sync ?? false;
  }
  const base = cap.device_id.replace(/:group\d+$/, "");
  return rgb.devices.some((device) => device.mb_rgb_sync && device.device_id.replace(/:group\d+$/, "") === base);
}

const PORT_SUFFIX = /:port\d+$/;

/** Connected ports of the same controller that can merge, lowest port first. */
export function mergeSiblings(cap: RgbDeviceCapabilities, caps: RgbDeviceCapabilities[]): string[] {
  if (!cap.merged_effect_modes?.length || !PORT_SUFFIX.test(cap.device_id)) return [];
  const base = cap.device_id.replace(PORT_SUFFIX, "");
  return caps
    .filter((other) => other.merged_effect_modes?.length && other.device_id.replace(PORT_SUFFIX, "") === base)
    .map((other) => other.device_id)
    .sort();
}

export function portsMerged(cap: RgbDeviceCapabilities, rgb: RgbAppConfig): boolean {
  if (!cap.merged_effect_modes?.length || !PORT_SUFFIX.test(cap.device_id)) return false;
  const base = cap.device_id.replace(PORT_SUFFIX, "");
  return rgb.devices.some((device) => device.port_merge?.enabled && device.device_id.replace(PORT_SUFFIX, "") === base);
}

/** Quick Sync on any sibling runs every port separately until it ends. */
export function mergeSuspended(cap: RgbDeviceCapabilities, caps: RgbDeviceCapabilities[], rgb: RgbAppConfig): boolean {
  if (!portsMerged(cap, rgb)) return false;
  const siblings = mergeSiblings(cap, caps);
  return caps.some((other) => siblings.includes(other.device_id) && quickSyncControls(other, rgb));
}

/** Ports listed first, then the rest in ascending order, matching the daemon. */
export function mergePortOrder(order: number[] | undefined, ports: number[]): number[] {
  const sorted = [...ports].sort((a, b) => a - b);
  return [...(order ?? []).filter((port) => sorted.includes(port)), ...sorted.filter((port) => !order?.includes(port))]
    .filter((port, index, all) => all.indexOf(port) === index);
}

export function quickSyncControls(cap: RgbDeviceCapabilities, rgb: RgbAppConfig): boolean {
  const sync = rgb.merge_lighting;
  if (rgb.devices.find((device) => device.device_id === cap.device_id)?.mb_rgb_sync) return false;
  if (!sync?.enabled || !sync.device_order.includes(cap.device_id)) return false;
  if (sync.disabled_devices.includes(cap.device_id)) return false;
  return sync.kind === "Continuous"
    ? cap.sync_led_count != null
    : cap.supported_modes.includes(sync.effect.mode);
}

export function createDefaultSync(caps: RgbDeviceCapabilities[]): MergeLightingConfig {
  const deviceOrder = caps.filter((cap) => cap.sync_led_count != null).map((cap) => cap.device_id);
  return {
    enabled: false,
    kind: "Continuous",
    effect_memory: [],
    device_order: deviceOrder,
    directions: deviceOrder.map(() => "Clockwise"),
    effect: {
      mode: "Rainbow",
      colors: [],
      speed: 2,
      brightness: 4,
      direction: "Clockwise",
      scope: "All",
      disabled: false,
    },
    disabled_devices: [],
  };
}

export function continuousParametersFor(caps: RgbDeviceCapabilities[]): RgbEffectParameters[] {
  const first = caps[0]?.sync_effect_parameters ?? [];
  return first.flatMap((entry) => {
    const parameters = caps.map((cap) =>
      cap.sync_effect_parameters?.find((candidate) => candidate.mode === entry.mode),
    );
    if (parameters.some((candidate) => !candidate)) return [];
    const intersection = intersectParameters(parameters as RgbEffectParameters[], entry.mode);
    return intersection ? [intersection] : [];
  });
}

export function matchedParametersFor(
  mode: string,
  caps: RgbDeviceCapabilities[],
): RgbEffectParameters | undefined {
  const base = matchedBaseParameters(mode);
  if (!base || !caps.length) return base;
  const parameters = caps.map((cap) => cap.effect_parameters?.find((entry) => entry.mode === mode));
  if (parameters.some((entry) => !entry)) return base;
  const controls = intersectParameters(parameters as RgbEffectParameters[], mode);
  return {
    ...base,
    directions: controls?.directions ?? [],
    supports_speed: base.supports_speed && (controls?.supports_speed ?? true),
  };
}

function matchedBaseParameters(mode: string): RgbEffectParameters | undefined {
  if (!MATCHED_MODES.includes(mode)) return undefined;
  const colors = mode === "Runway" ? 2 : ["Static", "Breathing", "Meteor"].includes(mode) ? 1 : 0;
  return {
    mode,
    min_colors: colors,
    max_colors: colors,
    per_fan_colors: false,
    directions: [],
    supports_speed: mode !== "Static",
  };
}

function intersectParameters(
  parameters: RgbEffectParameters[],
  mode: string,
): RgbEffectParameters | undefined {
  if (!parameters.length) return undefined;
  const minColors = Math.max(...parameters.map((entry) => entry.min_colors));
  const maxColors = Math.min(...parameters.map((entry) => entry.max_colors));
  return {
    mode,
    min_colors: minColors <= maxColors ? minColors : 0,
    max_colors: minColors <= maxColors ? maxColors : 0,
    per_fan_colors: false,
    directions: parameters[0].directions.filter((direction) =>
      parameters.every((entry) => entry.directions.includes(direction)),
    ),
    supports_speed: parameters.every((entry) => entry.supports_speed),
  };
}
