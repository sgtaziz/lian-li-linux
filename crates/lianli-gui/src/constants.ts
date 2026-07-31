import type { DeviceFamily, RgbMode } from "@/types";

export const FAMILY_DISPLAY: Record<DeviceFamily, string> = {
  Ene6k77: "UNI FAN SL/AL",
  TlFan: "UNI FAN TL",
  TlLcd: "UNI FAN TL LCD",
  Galahad2Trinity: "Galahad II Trinity",
  HydroShiftLcd: "HydroShift LCD",
  Galahad2Lcd: "Galahad II LCD",
  WirelessTx: "Wireless TX Dongle",
  WirelessRx: "Wireless RX Dongle",
  Slv3Lcd: "UNI FAN SL Wireless LCD",
  Slv3Led: "UNI FAN SL Wireless",
  Tlv2Lcd: "UNI FAN TL Wireless LCD",
  Tlv2Led: "UNI FAN TL Wireless",
  SlInf: "UNI FAN SL-INF Wireless",
  Clv1: "UNI FAN CL Wireless",
  HydroShift2Lcd: "HydroShift II LCD",
  Lancool207: "Lancool 207 Digital",
  UniversalScreen: 'Universal Screen 8.8"',
  HydroShift2LcdDesktop: "HydroShift II LCD (Desktop Mode)",
  Lancool207Desktop: "Lancool 207 Digital (Desktop Mode)",
  UniversalScreenDesktop: 'Universal Screen 8.8" (Desktop Mode)',
  WirelessAio: "HydroShift Wireless AIO",
  WirelessStrimer: "Strimer Plus Wireless",
  WirelessLc217: "Lancool 217 Wireless",
  WirelessLed88: 'Universal Screen 8.8" Wireless',
  WirelessV150: "Lancool V150 Wireless",
  StrimerPlus: "Strimer Plus",
  UniversalScreenLighting: 'Universal Screen 8.8" LED Ring',
  Vision9p2: "Vision 9.2\"",
  Vision9p2Desktop: 'Vision 9.2" (Desktop Mode)',
  TlFlexLcd: "TL Flex LCD",
  SlInfFlexLcd: "SL Infinity Flex LCD",
  WiredReceiver: "Wired Controller",
  HydroShift2OledCurveLcd: "HydroShift II OLED Curve",
  HydroShift2OledCurveLed: "HydroShift II OLED Curve LED",
};

// Families that should be hidden from the device list (the RF dongles).
export const DONGLE_FAMILIES: DeviceFamily[] = ["WirelessTx", "WirelessRx"];

// Capability predicates mirroring DeviceFamily::capabilities().
export function familyHasLcd(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("lcd");
}
export function familyHasFan(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("fan");
}
export function familyHasRgb(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("rgb");
}
export function familyHasPump(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("pump");
}
export function familyIsAio(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("aio");
}
export function familySupportsDisplaySwitch(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("display_mode_switch");
}
export function familyIsDesktopMode(f: DeviceFamily): boolean {
  return FAMILY_CAPS[f].includes("desktop_mode");
}

const FAMILY_CAPS: Record<DeviceFamily, string[]> = {
  Ene6k77: ["fan", "rgb"],
  TlFan: ["fan", "rgb"],
  StrimerPlus: ["rgb"],
  Galahad2Trinity: ["fan", "pump", "aio", "rgb"],
  HydroShiftLcd: ["fan", "pump", "aio", "rgb", "lcd"],
  Galahad2Lcd: ["fan", "pump", "aio", "rgb", "lcd"],
  TlLcd: ["lcd"],
  HydroShift2Lcd: ["lcd", "fan", "pump", "aio", "display_mode_switch"],
  Lancool207: ["lcd", "display_mode_switch"],
  UniversalScreen: ["lcd", "display_mode_switch"],
  Vision9p2: ["lcd", "display_mode_switch"],
  TlFlexLcd: ["lcd"],
  SlInfFlexLcd: ["lcd"],
  WiredReceiver: ["fan", "rgb"],
  HydroShift2OledCurveLcd: ["lcd", "display_mode_switch"],
  HydroShift2OledCurveLed: ["pump", "aio", "rgb"],
  HydroShift2LcdDesktop: ["lcd", "desktop_mode", "display_mode_switch"],
  Lancool207Desktop: ["lcd", "desktop_mode", "display_mode_switch"],
  UniversalScreenDesktop: ["lcd", "desktop_mode", "display_mode_switch"],
  Vision9p2Desktop: ["lcd", "desktop_mode", "display_mode_switch"],
  UniversalScreenLighting: ["rgb"],
  WirelessTx: ["wireless_dongle"],
  WirelessRx: ["wireless_dongle"],
  Slv3Lcd: ["fan", "lcd", "rgb"],
  Tlv2Lcd: ["fan", "lcd", "rgb"],
  Slv3Led: ["fan", "rgb"],
  Tlv2Led: ["fan", "rgb"],
  SlInf: ["fan", "rgb"],
  Clv1: ["fan", "rgb"],
  WirelessAio: ["fan", "pump", "aio", "rgb"],
  WirelessStrimer: ["rgb"],
  WirelessLc217: ["rgb"],
  WirelessLed88: ["rgb"],
  WirelessV150: ["rgb"],
};

// All RGB modes with display labels (mirrors RgbMode::display_name()).
export const RGB_MODES: { mode: RgbMode; label: string }[] = [
  { mode: "Off", label: "Off" },
  { mode: "Direct", label: "Direct" },
  { mode: "Static", label: "Static" },
  { mode: "Rainbow", label: "Rainbow" },
  { mode: "RainbowMorph", label: "Rainbow Morph" },
  { mode: "Breathing", label: "Breathing" },
  { mode: "Runway", label: "Runway" },
  { mode: "Meteor", label: "Meteor" },
  { mode: "ColorCycle", label: "Color Cycle" },
  { mode: "Staggered", label: "Staggered" },
  { mode: "Tide", label: "Tide" },
  { mode: "Mixing", label: "Mixing" },
  { mode: "Voice", label: "Voice" },
  { mode: "Door", label: "Door" },
  { mode: "Render", label: "Render" },
  { mode: "Ripple", label: "Ripple" },
  { mode: "Reflect", label: "Reflect" },
  { mode: "TailChasing", label: "Tail Chasing" },
  { mode: "Paint", label: "Paint" },
  { mode: "PingPong", label: "Ping Pong" },
  { mode: "Stack", label: "Stack" },
  { mode: "StackMulti", label: "Stack Multi" },
  { mode: "Neon", label: "Neon" },
  { mode: "CoverCycle", label: "Cover Cycle" },
  { mode: "Wave", label: "Wave" },
  { mode: "Racing", label: "Racing" },
  { mode: "Lottery", label: "Lottery" },
  { mode: "Intertwine", label: "Intertwine" },
  { mode: "MeteorShower", label: "Meteor Shower" },
  { mode: "Collide", label: "Collide" },
  { mode: "ElectricCurrent", label: "Electric Current" },
  { mode: "Kaleidoscope", label: "Kaleidoscope" },
  { mode: "BigBang", label: "Big Bang" },
  { mode: "Vortex", label: "Vortex" },
  { mode: "Pump", label: "Pump" },
  { mode: "ColorsMorph", label: "Colors Morph" },
  { mode: "TaiChi", label: "Tai Chi" },
  { mode: "CrossingOver", label: "Crossing Over" },
  { mode: "ColorfulStarryNight", label: "Colorful Starry Night" },
  { mode: "StaticStarryNight", label: "Static Starry Night" },
  { mode: "Bounce", label: "Bounce" },
  { mode: "TickerTape", label: "Ticker Tape" },
  { mode: "Fluctuation", label: "Fluctuation" },
  { mode: "Transmit", label: "Transmit" },
  { mode: "Burst", label: "Burst" },
  { mode: "MopUp", label: "Mop Up" },
  { mode: "PacMan", label: "PacMan" },
  { mode: "MeteorRainbow", label: "Meteor Rainbow" },
  { mode: "Spring", label: "Spring" },
  { mode: "Scan", label: "Scan" },
  { mode: "Contest", label: "Contest" },
  { mode: "Warning", label: "Warning" },
  { mode: "SpanningTeacups", label: "Spanning Teacups" },
  { mode: "Tornado", label: "Tornado" },
  { mode: "DoubleMeteor", label: "Double Meteor" },
  { mode: "MeteorContest", label: "Meteor Contest" },
  { mode: "MeteorMix", label: "Meteor Mix" },
  { mode: "ReturnArc", label: "Return Arc" },
  { mode: "DoubleArc", label: "Double Arc" },
  { mode: "HeartBeat", label: "Heart Beat" },
  { mode: "HeartBeatRunway", label: "Heart Beat Runway" },
  { mode: "Disco", label: "Disco" },
  { mode: "ColorfulCity", label: "Colorful City" },
  { mode: "Twinkle", label: "Twinkle" },
  { mode: "Groove", label: "Groove" },
  { mode: "Tunnel", label: "Tunnel" },
  { mode: "BreathingRainbow", label: "Breathing Rainbow" },
  { mode: "Snooker", label: "Snooker" },
  { mode: "BlowUp", label: "Blow Up" },
  { mode: "ShockWave", label: "Shock Wave" },
  { mode: "BulletStack", label: "Bullet Stack" },
  { mode: "Drizzling", label: "Drizzling" },
  { mode: "FadeOut", label: "Fade Out" },
  { mode: "ColorTransfer", label: "Color Transfer" },
  { mode: "CrossOver", label: "Cross Over" },
  { mode: "Parallel", label: "Parallel" },
];

export function modeLabel(mode: RgbMode): string {
  return RGB_MODES.find((m) => m.mode === mode)?.label ?? mode;
}

export const RGB_DIRECTIONS: { value: import("@/types").RgbDirection; label: string }[] = [
  { value: "Clockwise", label: "CW" },
  { value: "CounterClockwise", label: "CCW" },
  { value: "Up", label: "Up" },
  { value: "Down", label: "Down" },
  { value: "Spread", label: "Spread" },
  { value: "Gather", label: "Gather" },
];

export const RGB_SCOPES: { value: import("@/types").RgbScope; label: string }[] = [
  { value: "All", label: "All" },
  { value: "Top", label: "Top" },
  { value: "Bottom", label: "Bottom" },
  { value: "Inner", label: "Inner" },
  { value: "Outer", label: "Outer" },
];

export const BRIGHTNESS_OFF = 255;

export const RGB_BRIGHTNESS: { value: number; label: string }[] = [
  { value: BRIGHTNESS_OFF, label: "Off" },
  { value: 0, label: "Lowest" },
  { value: 1, label: "Lower" },
  { value: 2, label: "Normal" },
  { value: 3, label: "Higher" },
  { value: 4, label: "Highest" },
];
