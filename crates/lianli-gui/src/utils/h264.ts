import type { AppConfig, DeviceInfo } from "@/types";

export function h264Disabled(config: AppConfig, device: DeviceInfo | null | undefined): boolean {
  const key = device?.h264?.key;
  return key !== undefined && config.h264_disabled.includes(key);
}

export function setH264Disabled(config: AppConfig, key: string, disabled: boolean) {
  const others = config.h264_disabled.filter((item) => item !== key);
  config.h264_disabled = disabled ? [...others, key].sort() : others;
}
