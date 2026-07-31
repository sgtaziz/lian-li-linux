// TypeScript mirrors of the Rust shared types in crates/lianli-shared/src/.
// Serialization formats (serde rename rules) are preserved exactly so JSON
// round-trips against the daemon.

// ─── Device families ────────────────────────────────────────────────────────
export type DeviceFamily =
  | "Ene6k77"
  | "TlFan"
  | "TlLcd"
  | "Galahad2Trinity"
  | "HydroShiftLcd"
  | "Galahad2Lcd"
  | "WirelessTx"
  | "WirelessRx"
  | "Slv3Lcd"
  | "Slv3Led"
  | "Tlv2Lcd"
  | "Tlv2Led"
  | "SlInf"
  | "Clv1"
  | "HydroShift2Lcd"
  | "Lancool207"
  | "UniversalScreen"
  | "HydroShift2LcdDesktop"
  | "Lancool207Desktop"
  | "UniversalScreenDesktop"
  | "WirelessAio"
  | "WirelessStrimer"
  | "WirelessLc217"
  | "WirelessLed88"
  | "WirelessV150"
  | "StrimerPlus"
  | "UniversalScreenLighting"
  | "Vision9p2"
  | "Vision9p2Desktop"
  | "TlFlexLcd"
  | "SlInfFlexLcd"
  | "WiredReceiver"
  | "HydroShift2OledCurveLcd"
  | "HydroShift2OledCurveLed";

export type RGB = [number, number, number];
export type RGBA = [number, number, number, number];

export interface DeviceInfo {
  device_id: string;
  family: DeviceFamily;
  name: string;
  serial: string | null;
  vid: number;
  pid: number;
  has_lcd: boolean;
  has_fan: boolean;
  has_pump: boolean;
  has_rgb: boolean;
  has_pump_control?: boolean;
  fan_count: number | null;
  per_fan_control: boolean | null;
  mb_sync_support: boolean;
  rgb_zone_count: number | null;
  screen_width: number | null;
  screen_height: number | null;
  is_unbound_wireless: boolean;
  pump_rpm_range?: [number, number] | null;
  fan_quantity?: number | null;
  max_fan_quantity?: number | null;
  firmware_version?: string | null;
  supports_c_command?: boolean;
  port_index?: [number, number] | null;
}

export interface OpenRgbServerStatus {
  enabled: boolean;
  running: boolean;
  port: number | null;
  error: string | null;
}

export interface TelemetrySnapshot {
  fan_rpms: Record<string, number[]>;
  coolant_temps: Record<string, number>;
  streaming_active: boolean;
  openrgb_status: OpenRgbServerStatus;
}

export interface PollResult {
  connected: boolean;
  socket_path: string;
  devices: DeviceInfo[];
  telemetry: TelemetrySnapshot;
}

// ─── Media / sensors ────────────────────────────────────────────────────────
export type MediaType =
  | "image"
  | "video"
  | "color"
  | "gif"
  | "sensor"
  | "doublegauge"
  | "cooler"
  | "custom";

export type SensorSourceConfig =
  | { type: "constant"; value: number }
  | { type: "command"; cmd: string }
  | { type: "hwmon"; name: string; label: string; device_path?: string }
  | { type: "nvidia_gpu"; gpu_index?: number; metric?: "temp" | "usage" }
  | { type: "amd_gpu_usage"; card_index?: number }
  | { type: "wireless_coolant"; device_id: string }
  | { type: "cpu_usage" }
  | { type: "mem_usage" }
  | { type: "mem_used" }
  | { type: "mem_free" }
  | { type: "network_rx"; iface: string }
  | { type: "network_tx"; iface: string }
  | { type: "disk_read"; device: string }
  | { type: "disk_write"; device: string };

export interface SensorRange {
  max: number | null;
  color: RGB;
  alpha?: number;
}

export interface SensorDescriptor {
  label: string;
  unit: string;
  source: SensorSourceConfig;
  text_color: RGB;
  background_color: RGB;
  gauge_background_color: RGB;
  gauge_ranges: SensorRange[];
  update_interval_ms?: number;
  gauge_start_angle: number;
  gauge_sweep_angle: number;
  gauge_outer_radius: number;
  gauge_thickness: number;
  bar_corner_radius: number;
  value_font_size: number;
  unit_font_size: number;
  label_font_size: number;
  font_path: string | null;
  decimal_places: number;
  value_offset: number;
  unit_offset: number;
  label_offset: number;
}

export interface DoublegaugeDescriptor {
  header?: string;
  gauge_1_min?: number;
  gauge_1_max?: number;
  value_1_min?: number;
  value_1_max?: number;
  display_value_1_min?: number;
  display_value_1_max?: number;
  clamp_1?: boolean;
  unit_1?: string;
  label_1?: string;
  decimals_1?: number;
  gauge_2_min?: number;
  gauge_2_max?: number;
  value_2_min?: number;
  value_2_max?: number;
  display_value_2_min?: number;
  display_value_2_max?: number;
  clamp_2?: boolean;
  unit_2?: string;
  label_2?: string;
  decimals_2?: number;
}

export interface LcdConfig {
  index?: number | null;
  serial: string | null;
  type: MediaType;
  path: string | null;
  fps: number | null;
  update_interval_ms?: number | null;
  rgb: RGB | null;
  orientation: number;
  sensor?: SensorDescriptor | null;
  sensor_source_1?: SensorSourceConfig;
  sensor_source_2?: SensorSourceConfig;
  doublegauge?: DoublegaugeDescriptor | null;
  template_id?: string | null;
  smooth_edges?: boolean | null;
  custom_h264?: boolean | null;
  aio_512_frame?: boolean | null;
  brightness?: number | null;
}

// ─── Fans ────────────────────────────────────────────────────────────────────
export type SensorSource =
  | { type: "hwmon"; name: string; label: string; device_path?: string }
  | { type: "nvidia_gpu"; gpu_index?: number; metric?: "temp" | "usage" }
  | { type: "amd_gpu_usage"; card_index?: number }
  | { type: "command"; cmd: string }
  | { type: "wireless_coolant"; device_id: string }
  | { type: "cpu_usage" }
  | { type: "mem_usage" }
  | { type: "mem_used" }
  | { type: "mem_free" }
  | { type: "network_rate"; iface: string; direction: "rx" | "tx" }
  | { type: "disk_rate"; device: string; direction: "read" | "write" };

export type FanSpeed = number | string;

export interface FanCurve {
  name: string;
  temp_source?: SensorSource | null;
  temp_command?: string;
  curve: [number, number][];
}

export interface FanGroup {
  device_id?: string | null;
  speeds: FanSpeed[];
}

export interface FanConfig {
  speeds: FanGroup[];
  update_interval_ms: number;
  hysteresis_temp: number;
  hysteresis_pwm: number;
}

// ─── RGB ────────────────────────────────────────────────────────────────────
export type RgbMode = string;
export type RgbDirection = "Clockwise" | "CounterClockwise" | "Up" | "Down" | "Spread" | "Gather";
export type RgbScope = "All" | "Top" | "Bottom" | "Inner" | "Outer";

export interface RgbEffect {
  mode: RgbMode;
  colors: RGB[];
  speed: number;
  brightness: number;
  direction: RgbDirection;
  scope: RgbScope;
  disabled: boolean;
}

export interface RgbZoneConfig {
  zone_index: number;
  effect: RgbEffect;
  swap_lr: boolean;
  swap_tb: boolean;
}

export interface RgbDeviceConfig {
  device_id: string;
  mb_rgb_sync: boolean;
  active_preset?: string | null;
  zones: RgbZoneConfig[];
}

export interface RgbAppConfig {
  enabled: boolean;
  openrgb_server: boolean;
  openrgb_port: number;
  devices: RgbDeviceConfig[];
}

export interface RgbZoneInfo {
  name: string;
  led_count: number;
}

export interface RgbDeviceCapabilities {
  device_id: string;
  device_name: string;
  supported_modes: RgbMode[];
  zones: RgbZoneInfo[];
  supports_direct: boolean;
  supports_mb_rgb_sync: boolean;
  total_led_count: number;
  supported_scopes: RgbScope[][];
  supports_direction?: boolean;
  supports_merge_lighting?: boolean;
}

export interface RgbPresetZone {
  zone: number;
  colors?: RGB[];
  effect?: RgbEffect | null;
}

export interface RgbPreset {
  name: string;
  device_id: string;
  zones: RgbPresetZone[];
}

// ─── AIO ──────────────────────────────────────────────────────────────────────
export interface AioConfig {
  pump_target_rpm: FanSpeed;
  fan_speeds: FanSpeed[];
  theme_index: number;
  brightness: number;
  rotation: number;
  loop_interval: number;
  cpu_temp_source: SensorSourceConfig | null;
  cpu_load_source: SensorSourceConfig | null;
  gpu_temp_source: SensorSourceConfig | null;
  gpu_load_source: SensorSourceConfig | null;
  str_color: RGBA;
  val_color: RGBA;
  unit_color: RGBA;
}

// ─── Thermal alert ──────────────────────────────────────────────────────────
export interface ThermalAlertSourceSettings {
  enabled: boolean;
  threshold: number;
  alert_color: RGB;
}

export interface ThermalAlertSettings {
  cpu: ThermalAlertSourceSettings;
  gpu: ThermalAlertSourceSettings;
}

// ─── ENE 6K77 ───────────────────────────────────────────────────────────────
export interface Ene6k77DeviceConfig {
  fan_quantities: Record<string, number>;
}

// ─── AppConfig ───────────────────────────────────────────────────────────────
export interface AppConfig {
  default_fps: number;
  lcds: LcdConfig[];
  fan_curves: FanCurve[];
  fans: FanConfig | null;
  rgb: RgbAppConfig | null;
  aio: Record<string, AioConfig>;
  ene6k77: Record<string, Ene6k77DeviceConfig>;
  thermal_alert: ThermalAlertSettings;
  rgb_drift_detection_enabled: boolean;
  rgb_drift_detection_interval_ms: number;
}

// ─── Templates ───────────────────────────────────────────────────────────────
export type TextAlign = "left" | "center" | "right";
export type BarOrientation = "horizontal" | "vertical";
export type ImageFit = "stretch" | "contain" | "cover";

export interface FontRef {
  path?: string | null;
}

export interface GradientStop {
  position: number;
  color: RGB;
  alpha?: number;
}

export interface Widget {
  id: string;
  kind: any;
  x: number;
  y: number;
  width: number;
  height: number;
  rotation?: number;
  visible?: boolean;
  update_interval_ms?: number | null;
  fps?: number | null;
  sensor_category?: string | null;
}

export interface TemplateBackground {
  type: "color";
  rgb: RGBA;
}

export interface LcdTemplate {
  id: string;
  name: string;
  base_width: number;
  base_height: number;
  background: TemplateBackground;
  widgets: Widget[];
  rotated: boolean;
  target_device?: string | null;
}

// ─── Sensors ─────────────────────────────────────────────────────────────────
export type Unit = "C" | "RPM" | "V" | "FREQ" | "PERCENT" | "SIZE" | "MBps" | "WO";

export interface SensorInfo {
  source: SensorSource;
  sensor_name: { device_name: string; sensor_name: string } | null;
  display_name: string | null;
  divider: number;
  unit: Unit;
  current_value: number | null;
}

// ─── Catalog (template browser) ──────────────────────────────────────────────
export interface CatalogFile {
  path: string;
  sha256: string;
}

export interface CatalogTemplate {
  id: string;
  name: string;
  description?: string;
  author?: string;
  min_daemon_version: string;
  folder: string;
  template_file: string;
  template_sha256: string;
  preview: string;
  preview_sha256: string;
  base_width?: number;
  base_height?: number;
  rotated?: boolean;
  files?: CatalogFile[];
}

export interface CatalogManifest {
  schema_version: number;
  templates: CatalogTemplate[];
}

// ─── IPC helpers ─────────────────────────────────────────────────────────────
export type PendingActionKind = "bind" | "unbind" | "switch" | "fan-quantity";

export const MB_SYNC_KEY = "__mb_sync__";
export const MB_SYNC_PREFIX = "__mb_sync__:";

export interface PwmHeader {
  id: string;
  label: string;
}
