import type { RGB, RgbDeviceConfig, RgbEffect, RgbEffectMemory, RgbScope } from "@/types";

export function copyEffect(effect: RgbEffect): RgbEffect {
  return {
    ...effect,
    colors: effect.colors.map((color) => [...color] as RGB),
  };
}

export function rememberDeviceEffect(
  device: RgbDeviceConfig,
  zone: number | null,
  effect: RgbEffect,
  flip: boolean,
) {
  const memory = device.effect_memory ?? [];
  const entry: RgbEffectMemory = { zone, effect: copyEffect(effect), flip };
  device.effect_memory = memory.filter((candidate) => !sameDeviceEffect(candidate, zone, effect.scope, effect.mode));
  device.effect_memory.push(entry);
}

export function recallDeviceEffect(
  device: RgbDeviceConfig,
  zone: number | null,
  scope: RgbScope,
  mode: string,
): RgbEffectMemory | undefined {
  const entry = device.effect_memory?.find((candidate) => sameDeviceEffect(candidate, zone, scope, mode));
  if (!entry) return undefined;
  return { ...entry, effect: copyEffect(entry.effect) };
}

export function rememberSyncEffect(memory: RgbEffect[] | undefined, effect: RgbEffect): RgbEffect[] {
  return [...(memory ?? []).filter((candidate) => candidate.mode !== effect.mode), copyEffect(effect)];
}

export function recallSyncEffect(memory: RgbEffect[] | undefined, mode: string): RgbEffect | undefined {
  const effect = memory?.find((candidate) => candidate.mode === mode);
  return effect ? copyEffect(effect) : undefined;
}

function sameDeviceEffect(
  entry: RgbEffectMemory,
  zone: number | null,
  scope: RgbScope,
  mode: string,
): boolean {
  return (entry.zone ?? null) === zone && entry.effect.scope === scope && entry.effect.mode === mode;
}
