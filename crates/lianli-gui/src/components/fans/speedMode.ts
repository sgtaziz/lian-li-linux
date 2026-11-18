import type { FanCurve, FanGroup, FanSpeed, SensorSource } from "@/types";
import { MB_SYNC_KEY } from "@/types";

export type FanSlots = [FanSpeed, FanSpeed, FanSpeed, FanSpeed];

export function isMbSyncGroup(speeds: FanSpeed[]): boolean {
  return speeds.some((s) => typeof s === "string" && s.startsWith(MB_SYNC_KEY));
}

// MB Sync is port-wide, so any MB Sync slot puts the whole group in MB Sync.
export function speedModeOf(speeds: FanSpeed[], slot: number): string {
  if (isMbSyncGroup(speeds)) return MB_SYNC_KEY;
  const s = speeds[slot];
  if (typeof s === "number") return "constant";
  if (s === "off" || s === "") return "off";
  return `curve:${s}`;
}

export function decodeSpeedMode(value: string): FanSpeed {
  if (value === "off") return "off";
  if (value.startsWith("curve:")) return value.slice("curve:".length);
  return 128;
}

export function fillSlots(group: FanGroup, value: FanSpeed) {
  group.speeds = [value, value, value, value] as FanSlots;
}

// Mirrors FanSpeed::is_mb_sync in lianli-shared.
export function isMbSyncKey(name: string): boolean {
  return name === MB_SYNC_KEY || name.startsWith(`${MB_SYNC_KEY}:`);
}

// Mirrors FanSpeed::is_off and FanSpeed::is_mb_sync in lianli-shared.
export function isSpeedModeKey(name: string): boolean {
  return name === "off" || isMbSyncKey(name);
}

// Mirrors the group skip in the daemon's FanController: such groups run no curves at all.
export function groupRunsCurves(group: FanGroup): boolean {
  const wireless = group.device_id?.startsWith("wireless:") ?? false;
  return group.speeds.every((s) => s !== "off" && (wireless || typeof s !== "string" || !isMbSyncKey(s)));
}

// Stricter than the daemon so new names never look like MB Sync in the GUI.
export function isReservedCurveName(name: string): boolean {
  return name === "off" || name.startsWith(MB_SYNC_KEY);
}

const DEFAULT_CURVE_COMMAND = "cat /sys/class/thermal/thermal_zone0/temp | awk '{print $1/1000}'";

// Mirrors FanCurve::effective_source in lianli-shared.
export function effectiveCurveSource(curve: FanCurve): SensorSource {
  const command = curve.temp_command ?? "";
  if (curve.temp_source) return curve.temp_source.type === "command" ? { type: "command", cmd: command } : curve.temp_source;
  return { type: "command", cmd: command || DEFAULT_CURVE_COMMAND };
}

// Fills the fields the daemon's serde defaults supply, so a draft compares equal to its applied copy.
function normalizedSource(source: SensorSource): Record<string, unknown> {
  switch (source.type) {
    case "hwmon":
      return { ...source, device_path: source.device_path ?? "" };
    case "nvidia_gpu":
      return { ...source, gpu_index: source.gpu_index ?? 0, metric: source.metric ?? "temp" };
    case "amd_gpu_usage":
      return { ...source, card_index: source.card_index ?? 0 };
    default:
      return { ...source };
  }
}

export function sameSensorSource(a: SensorSource, b: SensorSource): boolean {
  const left = normalizedSource(a);
  const right = normalizedSource(b);
  const keys = new Set([...Object.keys(left), ...Object.keys(right)]);
  return [...keys].every((key) => left[key] === right[key]);
}
