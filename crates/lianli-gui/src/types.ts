// TypeScript types mirroring Rust structs in lianli-shared.
// Must match serde serialization exactly.

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
  | "DisplaySwitcher";

export interface DeviceInfo {
  device_id: string;
  family: DeviceFamily;
  name: string;
  serial: string | null;
  has_lcd: boolean;
  has_fan: boolean;
  has_pump: boolean;
  has_rgb: boolean;
  fan_count: number | null;
  per_fan_control: boolean | null;
  mb_sync_support: boolean;
  rgb_zone_count: number | null;
  screen_width: number | null;
  screen_height: number | null;
}

export type MediaType = "image" | "video" | "color" | "gif" | "sensor";

export interface SensorRange {
  max: number | null;
  color: [number, number, number];
}

export interface SensorSourceConfig {
  type: "constant" | "command";
  value?: number; // for constant
  cmd?: string; // for command
}

export interface SensorDescriptor {
  label: string;
  unit: string;
  source: SensorSourceConfig;
  text_color: [number, number, number];
  background_color: [number, number, number];
  gauge_background_color: [number, number, number];
  gauge_ranges: SensorRange[];
  update_interval_ms: number;
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

export interface LcdConfig {
  index?: number;
  serial?: string;
  type: MediaType; // serde(rename = "type")
  path?: string;
  fps?: number;
  rgb?: [number, number, number];
  orientation: number;
  sensor?: SensorDescriptor;
}

export type FanSpeed = number | string; // number = constant PWM, string = curve name

export interface FanCurve {
  name: string;
  temp_command: string;
  curve: [number, number][]; // [temp_celsius, speed_percent]
}

export interface FanGroup {
  device_id?: string;
  speeds: [FanSpeed, FanSpeed, FanSpeed, FanSpeed];
}

export interface FanConfig {
  speeds: FanGroup[];
  update_interval_ms: number;
}

export interface AppConfig {
  default_fps: number;
  lcds: LcdConfig[];
  fan_curves: FanCurve[];
  fans?: FanConfig;
  rgb?: RgbAppConfig;
}

// ─── RGB Types ───

export type RgbMode =
  | "Off" | "Direct" | "Static" | "Rainbow" | "RainbowMorph"
  | "Breathing" | "Runway" | "Meteor" | "ColorCycle" | "Staggered"
  | "Tide" | "Mixing" | "Voice" | "Door" | "Render"
  | "Ripple" | "Reflect" | "TailChasing" | "Paint" | "PingPong"
  | "Stack" | "CoverCycle" | "Wave" | "Racing" | "Lottery"
  | "Intertwine" | "MeteorShower" | "Collide" | "ElectricCurrent" | "Kaleidoscope"
  | "BigBang" | "Vortex" | "Pump" | "ColorsMorph";

export type RgbDirection =
  | "Clockwise" | "CounterClockwise" | "Up" | "Down" | "Spread" | "Gather";

export type RgbScope = "All" | "Top" | "Bottom" | "Inner" | "Outer";

export interface RgbEffect {
  mode: RgbMode;
  colors: [number, number, number][];
  speed: number;
  brightness: number;
  direction: RgbDirection;
  scope: RgbScope;
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
}

export const RGB_MODE_NAMES: Record<RgbMode, string> = {
  Off: "Off",
  Direct: "Direct",
  Static: "Static",
  Rainbow: "Rainbow",
  RainbowMorph: "Rainbow Morph",
  Breathing: "Breathing",
  Runway: "Runway",
  Meteor: "Meteor",
  ColorCycle: "Color Cycle",
  Staggered: "Staggered",
  Tide: "Tide",
  Mixing: "Mixing",
  Voice: "Voice",
  Door: "Door",
  Render: "Render",
  Ripple: "Ripple",
  Reflect: "Reflect",
  TailChasing: "Tail Chasing",
  Paint: "Paint",
  PingPong: "Ping Pong",
  Stack: "Stack",
  CoverCycle: "Cover Cycle",
  Wave: "Wave",
  Racing: "Racing",
  Lottery: "Lottery",
  Intertwine: "Intertwine",
  MeteorShower: "Meteor Shower",
  Collide: "Collide",
  ElectricCurrent: "Electric Current",
  Kaleidoscope: "Kaleidoscope",
  BigBang: "Big Bang",
  Vortex: "Vortex",
  Pump: "Pump",
  ColorsMorph: "Colors Morph",
};

export interface TelemetrySnapshot {
  fan_rpms: Record<string, number[]>;
  coolant_temps: Record<string, number>;
  streaming_active: boolean;
}

// Helper: human-readable family names
export const FAMILY_NAMES: Record<DeviceFamily, string> = {
  Ene6k77: "UNI FAN SL/AL",
  TlFan: "UNI FAN TL",
  TlLcd: "UNI FAN TL LCD",
  Galahad2Trinity: "Galahad II Trinity AIO",
  HydroShiftLcd: "HydroShift LCD AIO",
  Galahad2Lcd: "Galahad II LCD AIO",
  WirelessTx: "Wireless TX Dongle",
  WirelessRx: "Wireless RX Dongle",
  Slv3Lcd: "UNI FAN SL Wireless LCD",
  Slv3Led: "UNI FAN SL Wireless",
  Tlv2Lcd: "UNI FAN TL Wireless LCD",
  Tlv2Led: "UNI FAN TL Wireless",
  SlInf: "UNI FAN SL-INF Wireless",
  Clv1: "UNI FAN CL Wireless",
  HydroShift2Lcd: "HydroShift II LCD Circle",
  Lancool207: "Lancool 207 Digital",
  UniversalScreen: 'Universal Screen 8.8"',
  DisplaySwitcher: "Display Mode Switcher",
};
