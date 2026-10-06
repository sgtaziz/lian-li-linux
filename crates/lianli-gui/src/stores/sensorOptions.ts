import type { SensorInfo, SensorSource, SensorSourceConfig } from "@/types";

export interface SelectOption {
  label: string;
  value: string;
}

/**
 * Build NSelect options from the enumerated sensor list.
 *
 * When `includeCommand` is true, a "Custom command" sentinel option is
 * appended (value "command") so fan curves can fall back to a shell command.
 *
 * When `tempOnly` is true, only temperature sensors (unit "C") are included —
 * use this for fan curve temperature sources where %, RPM, etc. are invalid.
 */
export function enumerateSensorsAsOptions(
  sensors: SensorInfo[],
  includeCommand: boolean,
  tempOnly: boolean = false,
): SelectOption[] {
  const filtered = tempOnly ? sensors.filter((s) => s.unit === "C") : sensors;
  const opts: SelectOption[] = filtered.map((s) => ({
    label: formatSensorLabel(s),
    value: JSON.stringify(s.source),
  }));
  if (includeCommand) {
    opts.push({ label: "Custom command", value: "command" });
  }
  return opts;
}

export function sourceConfigsAsOptions(
  sensors: SensorInfo[],
  includeCommand: boolean = false,
): SelectOption[] {
  const options = sensors.map((s) => ({
    label: formatSensorLabel(s),
    value: JSON.stringify(sourceToConfig(s.source)),
  }));
  if (includeCommand) options.push({ label: "Custom command", value: "command" });
  return options;
}

export function sourceToConfig(source: SensorSource | SensorSourceConfig): SensorSourceConfig {
  switch (source.type) {
    case "network_rate":
      return { type: source.direction === "rx" ? "network_rx" : "network_tx", iface: source.iface };
    case "disk_rate":
      return { type: source.direction === "read" ? "disk_read" : "disk_write", device: source.device };
    default:
      return source;
  }
}

function formatSensorLabel(s: SensorInfo): string {
  const unit = UNIT_LABELS[s.unit];
  if (s.display_name) {
    return unit ? `${s.display_name} (${unit})` : s.display_name;
  }
  const sensor = s.sensor_name?.sensor_name ?? "sensor";
  const device = s.sensor_name?.device_name;
  let name = device && device !== sensor ? `${device}: ${sensor}` : sensor;
  if (s.source?.type === "hwmon" && s.source.name && !name.includes(s.source.name)) {
    name = `${name} [${s.source.name}]`;
  }
  return unit ? `${name} (${unit})` : name;
}

const GAUGE_LABELS: Record<string, string> = {
  cpu_usage: "CPU",
  cpu_temp: "CPU",
  mem_usage: "RAM",
  mem_used: "RAM Used",
  mem_free: "RAM Free",
  gpu_usage: "GPU",
  gpu_temp: "GPU",
  network_rx: "Net RX",
  network_tx: "Net TX",
  disk_read: "Disk Read",
  disk_write: "Disk Write",
};

const MAX_GAUGE_LABEL_LENGTH = 16;

export function gaugeTextForSource(
  sensors: SensorInfo[],
  cfg: SensorSourceConfig,
): { label: string; unit: string } | null {
  const json = JSON.stringify(cfg);
  const sensor = sensors.find((s) => JSON.stringify(sourceToConfig(s.source)) === json);
  if (!sensor) return null;
  const category = inferSensorCategory(cfg);
  const fallback = sensor.sensor_name?.sensor_name ?? sensor.display_name ?? "";
  const label = (category ? GAUGE_LABELS[category] : undefined) ?? fallback;
  return {
    label: label.slice(0, MAX_GAUGE_LABEL_LENGTH),
    unit: UNIT_LABELS[sensor.unit] ?? "",
  };
}

const UNIT_LABELS: Record<string, string> = {
  C: "\u00b0C",
  RPM: "RPM",
  V: "mV",
  FREQ: "MHz",
  PERCENT: "%",
  SIZE: "GB",
  MBps: "MB/s",
  WO: "",
};

/** Decode a selected option value back into a SensorSourceConfig. */
export function decodeOption(value: string): SensorSourceConfig | null {
  if (!value || value === "command") return null;
  try {
    return sourceToConfig(JSON.parse(value));
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
  const sensor = sensors.find((s) => JSON.stringify(sourceToConfig(s.source)) === json);
  return sensor ? JSON.stringify(sensor.source) : "";
}

export function inferSensorCategory(src: SensorSourceConfig): string | null {
  switch (src.type) {
    case "cpu_usage":
      return "cpu_usage";
    case "mem_usage":
      return "mem_usage";
    case "mem_used":
      return "mem_used";
    case "mem_free":
      return "mem_free";
    case "nvidia_gpu":
      return src.metric === "usage" ? "gpu_usage" : "gpu_temp";
    case "amd_gpu_usage":
      return "gpu_usage";
    case "network_rx":
      return "network_rx";
    case "network_tx":
      return "network_tx";
    case "disk_read":
      return "disk_read";
    case "disk_write":
      return "disk_write";
    case "hwmon": {
      const l = src.label.toLowerCase();
      if (src.name === "k10temp" || src.name === "coretemp") return "cpu_temp";
      if ((src.name === "amdgpu" || src.name === "radeon") && (l.includes("edge") || l.includes("temp"))) {
        return "gpu_temp";
      }
      return null;
    }
    default:
      return null;
  }
}
