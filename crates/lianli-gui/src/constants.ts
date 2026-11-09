import type { DeviceFamily, RgbDirection, RgbMode, RgbScope } from "@/types";
import { tr, type MessageKey } from "@/i18n";

export const FAMILY_DISPLAY: Record<DeviceFamily, string> = {
  EdgeHub: "Edge Hub Advanced",
  Ene6k77: "UNI FAN SL/AL",
  TlFan: "UNI FAN TL",
  TlLcd: "UNI FAN TL LCD",
  Galahad2Trinity: "Galahad II Trinity",
  HydroShiftLcd: "HydroShift LCD",
  Galahad2Lcd: "Galahad II LCD",
  get WirelessTx() {
    return tr("devices.family.wirelessTx");
  },
  get WirelessRx() {
    return tr("devices.family.wirelessRx");
  },
  Slv3Lcd: "UNI FAN SL Wireless LCD",
  Slv3Led: "UNI FAN SL Wireless",
  Tlv2Lcd: "UNI FAN TL Wireless LCD",
  Tlv2Led: "UNI FAN TL Wireless",
  SlInf: "UNI FAN SL-INF Wireless",
  Clv1: "UNI FAN CL Wireless",
  HydroShift2Lcd: "HydroShift II LCD",
  Lancool207: "Lancool 207 Digital",
  UniversalScreen: 'Universal Screen 8.8"',
  get HydroShift2LcdDesktop() {
    return tr("devices.family.desktopMode", { name: "HydroShift II LCD" });
  },
  get Lancool207Desktop() {
    return tr("devices.family.desktopMode", { name: "Lancool 207 Digital" });
  },
  get UniversalScreenDesktop() {
    return tr("devices.family.desktopMode", { name: 'Universal Screen 8.8"' });
  },
  WirelessAio: "HydroShift Wireless AIO",
  WirelessStrimer: "Strimer Plus Wireless",
  WirelessLc217: "Lancool 217 Wireless",
  WirelessLed88: 'Universal Screen 8.8" Wireless',
  WirelessV150: "Lancool V150 Wireless",
  StrimerPlus: "Strimer Plus",
  UniversalScreenLighting: 'Universal Screen 8.8" LED Ring',
  Vision9p2: "Vision 9.2\"",
  get Vision9p2Desktop() {
    return tr("devices.family.desktopMode", { name: 'Vision 9.2"' });
  },
  TlFlexLcd: "TL Flex LCD",
  SlInfFlexLcd: "SL Infinity Flex LCD",
  get WiredReceiver() {
    return tr("devices.family.wiredReceiver");
  },
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
  EdgeHub: ["temperature"],
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
const RGB_MODE_KEYS: Record<RgbMode, MessageKey> = {
  Off: "rgb.modes.off",
  Direct: "rgb.modes.direct",
  Static: "rgb.modes.static",
  Rainbow: "rgb.modes.rainbow",
  RainbowMorph: "rgb.modes.rainbowMorph",
  Breathing: "rgb.modes.breathing",
  Runway: "rgb.modes.runway",
  Meteor: "rgb.modes.meteor",
  ColorCycle: "rgb.modes.colorCycle",
  Staggered: "rgb.modes.staggered",
  Tide: "rgb.modes.tide",
  Mixing: "rgb.modes.mixing",
  Voice: "rgb.modes.voice",
  Door: "rgb.modes.door",
  Render: "rgb.modes.render",
  Ripple: "rgb.modes.ripple",
  Reflect: "rgb.modes.reflect",
  TailChasing: "rgb.modes.tailChasing",
  Paint: "rgb.modes.paint",
  PingPong: "rgb.modes.pingPong",
  Stack: "rgb.modes.stack",
  StackMulti: "rgb.modes.stackMulti",
  Neon: "rgb.modes.neon",
  CoverCycle: "rgb.modes.coverCycle",
  Wave: "rgb.modes.wave",
  Racing: "rgb.modes.racing",
  Lottery: "rgb.modes.lottery",
  Intertwine: "rgb.modes.intertwine",
  MeteorShower: "rgb.modes.meteorShower",
  Collide: "rgb.modes.collide",
  ElectricCurrent: "rgb.modes.electricCurrent",
  Kaleidoscope: "rgb.modes.kaleidoscope",
  BigBang: "rgb.modes.bigBang",
  Vortex: "rgb.modes.vortex",
  Pump: "rgb.modes.pump",
  ColorsMorph: "rgb.modes.colorsMorph",
  TaiChi: "rgb.modes.taiChi",
  CrossingOver: "rgb.modes.crossingOver",
  ColorfulStarryNight: "rgb.modes.colorfulStarryNight",
  StaticStarryNight: "rgb.modes.staticStarryNight",
  Bounce: "rgb.modes.bounce",
  TickerTape: "rgb.modes.tickerTape",
  Fluctuation: "rgb.modes.fluctuation",
  Transmit: "rgb.modes.transmit",
  Burst: "rgb.modes.burst",
  MopUp: "rgb.modes.mopUp",
  PacMan: "rgb.modes.pacMan",
  MeteorRainbow: "rgb.modes.meteorRainbow",
  Spring: "rgb.modes.spring",
  Scan: "rgb.modes.scan",
  Contest: "rgb.modes.contest",
  Warning: "rgb.modes.warning",
  SpanningTeacups: "rgb.modes.spanningTeacups",
  Tornado: "rgb.modes.tornado",
  DoubleMeteor: "rgb.modes.doubleMeteor",
  MeteorContest: "rgb.modes.meteorContest",
  MeteorMix: "rgb.modes.meteorMix",
  ReturnArc: "rgb.modes.returnArc",
  DoubleArc: "rgb.modes.doubleArc",
  HeartBeat: "rgb.modes.heartBeat",
  HeartBeatRunway: "rgb.modes.heartBeatRunway",
  Disco: "rgb.modes.disco",
  ColorfulCity: "rgb.modes.colorfulCity",
  Twinkle: "rgb.modes.twinkle",
  Endless: "rgb.modes.endless",
  River: "rgb.modes.river",
  Duel: "rgb.modes.duel",
  Hourglass: "rgb.modes.hourglass",
  Pioneer: "rgb.modes.pioneer",
  ShuttleRun: "rgb.modes.shuttleRun",
  GradientRibbon: "rgb.modes.gradientRibbon",
  RainbowWave: "rgb.modes.rainbowWave",
  ColorfulMeteor: "rgb.modes.colorfulMeteor",
  Wing: "rgb.modes.wing",
  Drumming: "rgb.modes.drumming",
  Boomerang: "rgb.modes.boomerang",
  CandyBox: "rgb.modes.candyBox",
  Transformation: "rgb.modes.transformation",
  Nebula: "rgb.modes.nebula",
  Cinder: "rgb.modes.cinder",
  Pulsar: "rgb.modes.pulsar",
  NebulaRandom: "rgb.modes.nebulaRandom",
  CinderRandom: "rgb.modes.cinderRandom",
  PulsarRandom: "rgb.modes.pulsarRandom",
  Groove: "rgb.modes.groove",
  Tunnel: "rgb.modes.tunnel",
  BreathingRainbow: "rgb.modes.breathingRainbow",
  StaticColorful: "rgb.modes.staticColorful",
  BreathingColorful: "rgb.modes.breathingColorful",
  Snooker: "rgb.modes.snooker",
  BlowUp: "rgb.modes.blowUp",
  ShockWave: "rgb.modes.shockWave",
  BulletStack: "rgb.modes.bulletStack",
  Drizzling: "rgb.modes.drizzling",
  FadeOut: "rgb.modes.fadeOut",
  ColorTransfer: "rgb.modes.colorTransfer",
  CrossOver: "rgb.modes.crossOver",
  Parallel: "rgb.modes.parallel",
  SeaFlow: "rgb.modes.seaFlow",
  Echo: "rgb.modes.echo",
};

function translatedOptions<T>(keys: [T, MessageKey][]): { value: T; readonly label: string }[] {
  return keys.map(([value, key]) => ({
    value,
    get label() {
      return tr(key);
    },
  }));
}

export const RGB_MODES: { mode: RgbMode; readonly label: string }[] = Object.entries(RGB_MODE_KEYS).map(([mode, key]) => ({
  mode,
  get label() {
    return tr(key);
  },
}));

export function modeLabel(mode: RgbMode): string {
  const key = RGB_MODE_KEYS[mode];
  return key ? tr(key) : mode;
}

export const RGB_DIRECTIONS = translatedOptions<RgbDirection>([
  ["Clockwise", "rgb.directions.clockwise"],
  ["CounterClockwise", "rgb.directions.counterClockwise"],
  ["Up", "rgb.directions.up"],
  ["Down", "rgb.directions.down"],
  ["Spread", "rgb.directions.spread"],
  ["Gather", "rgb.directions.gather"],
]);

const RGB_SCOPE_KEYS: Record<RgbScope, MessageKey> = {
  All: "rgb.scopes.all",
  Fan: "rgb.scopes.fan",
  Top: "rgb.scopes.top",
  Bottom: "rgb.scopes.bottom",
  Inner: "rgb.scopes.inner",
  Outer: "rgb.scopes.outer",
  Center: "rgb.scopes.center",
  Pump: "rgb.scopes.pump",
  Front: "rgb.scopes.front",
  Rear: "rgb.scopes.rear",
  Segment1: "rgb.scopes.segment1",
  Segment2: "rgb.scopes.segment2",
  Segment3: "rgb.scopes.segment3",
  Segment4: "rgb.scopes.segment4",
  Segment5: "rgb.scopes.segment5",
  Segment6: "rgb.scopes.segment6",
};

export function scopeLabel(scope: RgbScope): string {
  const key = RGB_SCOPE_KEYS[scope];
  return key ? tr(key) : scope;
}

export const RGB_SCOPES = translatedOptions<RgbScope>([
  ["All", RGB_SCOPE_KEYS.All],
  ["Top", RGB_SCOPE_KEYS.Top],
  ["Bottom", RGB_SCOPE_KEYS.Bottom],
  ["Inner", RGB_SCOPE_KEYS.Inner],
  ["Outer", RGB_SCOPE_KEYS.Outer],
]);

export const BRIGHTNESS_OFF = 255;

export const RGB_BRIGHTNESS = translatedOptions<number>([
  [BRIGHTNESS_OFF, "common.off"],
  [0, "rgb.brightness.lowest"],
  [1, "rgb.brightness.lower"],
  [2, "rgb.brightness.normal"],
  [3, "rgb.brightness.higher"],
  [4, "rgb.brightness.highest"],
]);

export const PIXEL_CLEANER_DURATION_OPTIONS: { value: number; labelKey: MessageKey }[] = [
  { labelKey: "lcd.cleaner.durations.quick", value: 15 },
  { labelKey: "lcd.cleaner.durations.recommended", value: 30 },
  { labelKey: "lcd.cleaner.durations.deep", value: 60 },
  { labelKey: "lcd.cleaner.durations.heavy", value: 120 },
];
