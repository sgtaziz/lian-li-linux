// Screen preset labels mirroring lianli_shared::screen::screen_presets().
export interface ScreenPreset {
  label: string;
  width: number;
  height: number;
}

export const screenPresets: ScreenPreset[] = [
  { label: "Wireless LCD / TL LCD (400×400)", width: 400, height: 400 },
  { label: "AIO LCD / HydroShift 2 (480×480)", width: 480, height: 480 },
  { label: "HydroShift II OLED Curve (1080×2288)", width: 1080, height: 2288 },
  { label: "Lancool 207 (1472×720)", width: 1472, height: 720 },
  { label: 'Universal Screen 8.8" (480×1920)', width: 480, height: 1920 },
  { label: 'Vision 9.2" (464×1920)', width: 464, height: 1920 },
  { label: "Flex LCD (480×480)", width: 480, height: 480 },
];

import type { DeviceFamily } from "@/types";

/**
 * Whether a device family's LCD supports H.264 — mirrors
 * `lianli_shared::screen::screen_info_for(family).h264`.
 */
export function screenSupportsH264(family: DeviceFamily): boolean {
  return (
    family === "HydroShiftLcd" ||
    family === "Galahad2Lcd" ||
    family === "HydroShift2Lcd" ||
    family === "HydroShift2OledCurveLcd" ||
    family === "UniversalScreen"
  );
}

