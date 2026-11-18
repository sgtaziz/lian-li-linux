import type { SensorCategory, SensorInfo, SensorSource, SensorSourceConfig } from "@/types";
import { tr, type MessageKey } from "@/i18n";

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
    opts.push({ label: tr("fans.sensorOptions.customCommand"), value: "command" });
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
  if (includeCommand) options.push({ label: tr("fans.sensorOptions.customCommand"), value: "command" });
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

export function formatSensorLabel(s: SensorInfo): string {
  const unit = UNIT_LABELS[s.unit];
  if (s.display_name) {
    return unit ? `${s.display_name} (${unit})` : s.display_name;
  }
  const sensor = s.sensor_name?.sensor_name ?? tr("fans.sensorOptions.sensorFallback");
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
  gpu_mem_usage: "VRAM",
  gpu_mem_free: "VRAM Free",
  gpu_mem_total: "VRAM Total",
  gpu_clock_core: "GPU Clock",
  gpu_clock_memory: "VRAM Clock",
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
  MiB: "MiB",
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

export const SENSOR_CATEGORY_LABELS: Record<SensorCategory, MessageKey> = {
  cpu_temp: "editor.sensorBinding.categories.cpuTemp",
  gpu_temp: "editor.sensorBinding.categories.gpuTemp",
  cpu_usage: "editor.sensorBinding.categories.cpuUsage",
  gpu_usage: "editor.sensorBinding.categories.gpuUsage",
  gpu_mem_usage: "editor.sensorBinding.categories.gpuMemUsage",
  gpu_mem_free: "editor.sensorBinding.categories.gpuMemFree",
  gpu_mem_total: "editor.sensorBinding.categories.gpuMemTotal",
  gpu_clock_core: "editor.sensorBinding.categories.gpuClockCore",
  gpu_clock_memory: "editor.sensorBinding.categories.gpuClockMemory",
  mem_usage: "editor.sensorBinding.categories.memUsage",
  mem_used: "editor.sensorBinding.categories.memUsed",
  mem_free: "editor.sensorBinding.categories.memFree",
  network_rx: "editor.sensorBinding.categories.networkRx",
  network_tx: "editor.sensorBinding.categories.networkTx",
  disk_read: "editor.sensorBinding.categories.diskRead",
  disk_write: "editor.sensorBinding.categories.diskWrite",
};

export function isSensorCategory(value: unknown): value is SensorCategory {
  return typeof value === "string" && Object.prototype.hasOwnProperty.call(SENSOR_CATEGORY_LABELS, value);
}

/** Compares source configs regardless of the key order they were serialized with. */
export function sourceKey(config: SensorSourceConfig): string {
  return JSON.stringify(config, Object.keys(config).sort());
}

export function inferSensorCategory(src: SensorSourceConfig): SensorCategory | null {
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
      switch (src.metric) {
        case "usage":
          return "gpu_usage";
        case "mem_usage":
          return "gpu_mem_usage";
        case "mem_free":
          return "gpu_mem_free";
        case "mem_total":
          return "gpu_mem_total";
        case "clock_core":
          return "gpu_clock_core";
        case "clock_memory":
          return "gpu_clock_memory";
        default:
          return "gpu_temp";
      }
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
