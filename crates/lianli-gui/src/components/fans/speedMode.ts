import type { FanGroup, FanSpeed } from "@/types";
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

export function isReservedCurveName(name: string): boolean {
  return name === "" || name === "off" || name.startsWith(MB_SYNC_KEY);
}

// Mirrors FanSpeed::is_off and FanSpeed::is_mb_sync in lianli-shared.
export function isSpeedModeKey(name: string): boolean {
  return name === "off" || name === MB_SYNC_KEY || name.startsWith(`${MB_SYNC_KEY}:`);
}
