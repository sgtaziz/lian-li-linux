import type { SensorInfo, SensorSourceConfig } from "@/types";

export interface SelectOption {
  label: string;
  value: string;
}

/**
 * Build NSelect options from the enumerated sensor list.
 *
 * When `includeCommand` is true, a "Custom command" sentinel option is
 * appended (value "command") so fan curves can fall back to a shell command.
 */
export function enumerateSensorsAsOptions(
  sensors: SensorInfo[],
  includeCommand: boolean,
): SelectOption[] {
  const opts: SelectOption[] = sensors.map((s) => ({
    label: s.display_name ?? s.sensor_name?.sensor_name ?? "sensor",
    value: JSON.stringify(s.source),
  }));
  if (includeCommand) {
    opts.push({ label: "Custom command", value: "command" });
  }
  return opts;
}

/** Build options from SensorSourceConfig[] (e.g. for AIO source dropdowns). */
export function sourceConfigsAsOptions(
  sensors: SensorInfo[],
): SelectOption[] {
  return sensors.map((s) => ({
    label: s.display_name ?? s.sensor_name?.sensor_name ?? "sensor",
    value: JSON.stringify(s.source),
  }));
}

/** Decode a selected option value back into a SensorSourceConfig. */
export function decodeOption(value: string): SensorSourceConfig | null {
  if (!value || value === "command") return null;
  try {
    return JSON.parse(value) as SensorSourceConfig;
  } catch {
    return null;
  }
}

/** Find the option value matching a stored config (for initial selection). */
export function optionForConfig(
  sensors: SensorInfo[],
  cfg: SensorSourceConfig | null | undefined,
): string {
  if (!cfg) return "";
  const json = JSON.stringify(cfg);
  return sensors.some((s) => JSON.stringify(s.source) === json) ? json : "";
}
