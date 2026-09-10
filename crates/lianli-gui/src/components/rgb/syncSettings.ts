import type {
  MergeLightingConfig,
  RgbDeviceCapabilities,
  RgbEffectParameters,
} from "@/types";

export const MATCHED_MODES = ["Rainbow", "RainbowMorph", "Static", "Breathing", "Meteor", "Runway"];

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
