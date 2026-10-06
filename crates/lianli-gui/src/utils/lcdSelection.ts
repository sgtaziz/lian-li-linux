import type { DeviceInfo, LcdConfig } from "@/types";

export function resolveLcdDevice(entry: LcdConfig, devices: DeviceInfo[]): DeviceInfo | undefined {
  if (entry.serial) {
    const wanted = entry.serial.replace(/^hid:/, "");
    return devices.find((device) => device.device_id.replace(/^hid:/, "") === wanted);
  }
  return devices[entry.index ?? 0];
}

/** Display names that tell identical screens apart with a stable `#n` suffix. */
export function lcdDeviceLabels(devices: DeviceInfo[]): Map<string, string> {
  const sorted = [...devices].sort((a, b) => a.device_id.localeCompare(b.device_id));
  const totals = new Map<string, number>();
  for (const device of sorted) totals.set(device.name, (totals.get(device.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  const labels = new Map<string, string>();
  for (const device of sorted) {
    const n = (seen.get(device.name) ?? 0) + 1;
    seen.set(device.name, n);
    labels.set(device.device_id, (totals.get(device.name) ?? 1) > 1 ? `${device.name} #${n}` : device.name);
  }
  return labels;
}

const ORDER_KEY = "lianli.lcdOrder";

export function lcdEntryKey(entry: LcdConfig): string {
  return entry.serial ? `serial:${entry.serial.replace(/^hid:/, "")}` : `index:${entry.index ?? 0}`;
}

export function loadLcdOrder(): string[] {
  try {
    const stored = JSON.parse(localStorage.getItem(ORDER_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((key) => typeof key === "string") : [];
  } catch {
    return [];
  }
}

export function saveLcdOrder(keys: string[]) {
  try {
    localStorage.setItem(ORDER_KEY, JSON.stringify(keys));
  } catch {
    // Ordering is a display preference; the list falls back to config order.
  }
}

/** Config indices in the user's display order; unknown entries keep config order at the end. */
export function orderedLcdIndices(entries: LcdConfig[], order: string[]): number[] {
  const rank = new Map(order.map((key, position) => [key, position]));
  const rankOf = (index: number) => rank.get(lcdEntryKey(entries[index])) ?? order.length + index;
  return entries.map((_, index) => index).sort((a, b) => rankOf(a) - rankOf(b));
}

export function hasSavedLcdDevice(deviceId: string, saved: LcdConfig[], devices: DeviceInfo[]): boolean {
  return saved.some((entry) => resolveLcdDevice(entry, devices)?.device_id === deviceId);
}
