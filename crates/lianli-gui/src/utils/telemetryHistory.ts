export const HISTORY_WINDOW_MS = 5 * 60 * 1000;
const MAX_SAMPLES = 300;

export interface Sample {
  at: number;
  value: number;
}

export interface DeviceHistory {
  rpm: Sample[];
  coolant: Sample[];
}

export function averageSpinningRpm(rpms: number[]): number | null {
  if (!rpms.length) return null;
  const spinning = rpms.filter((rpm) => rpm > 0);
  if (!spinning.length) return 0;
  return spinning.reduce((sum, rpm) => sum + rpm, 0) / spinning.length;
}

function append(samples: Sample[], value: number | null, at: number): Sample[] {
  const kept = samples.filter((sample) => at - sample.at <= HISTORY_WINDOW_MS);
  if (value !== null && Number.isFinite(value)) kept.push({ at, value });
  return kept.length > MAX_SAMPLES ? kept.slice(kept.length - MAX_SAMPLES) : kept;
}

/** Adds one reading per device and drops devices that are no longer reported. */
export function recordHistory(
  previous: Record<string, DeviceHistory>,
  deviceIds: string[],
  fanRpms: Record<string, number[]>,
  coolantTemps: Record<string, number>,
  at: number,
): Record<string, DeviceHistory> {
  const next: Record<string, DeviceHistory> = {};
  for (const id of deviceIds) {
    const before = previous[id] ?? { rpm: [], coolant: [] };
    const entry = {
      rpm: append(before.rpm, averageSpinningRpm(fanRpms[id] ?? []), at),
      coolant: append(before.coolant, coolantTemps[id] ?? null, at),
    };
    if (entry.rpm.length || entry.coolant.length) next[id] = entry;
  }
  return next;
}
