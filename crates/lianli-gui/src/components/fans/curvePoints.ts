export type CurvePoint = [number, number];

// The chart shows this range by default and widens it to fit points outside it.
export const CURVE_TEMP_MIN = 20;
export const CURVE_TEMP_MAX = 100;
const CURVE_AXIS_STEP = 10;
// Matches the curve file reader in src-tauri/src/curve_files.rs.
export const CURVE_POINT_TEMP_MIN = 0;
export const CURVE_POINT_TEMP_MAX = 120;
export const MAX_CURVE_POINTS = 32;
export const MAX_CURVE_NAME_CHARS = 64;

export function sortCurve(points: CurvePoint[]): CurvePoint[] {
  return [...points].sort((a, b) => a[0] - b[0]);
}

// Mirrors interpolate_curve in lianli-shared.
export function speedAt(points: CurvePoint[], temp: number): number {
  const sorted = sortCurve(points);
  if (!sorted.length) return 100;
  if (temp <= sorted[0][0]) return sorted[0][1];
  const last = sorted[sorted.length - 1];
  if (temp >= last[0]) return last[1];
  for (let i = 1; i < sorted.length; i++) {
    const [t1, s1] = sorted[i - 1];
    const [t2, s2] = sorted[i];
    if (temp <= t2) return s1 + ((temp - t1) / (t2 - t1)) * (s2 - s1);
  }
  return last[1];
}

export interface CurveAxisRange {
  min: number;
  max: number;
}

/** The chart's temperature range: the default range, widened to the axis step around every point. */
export function curveAxisRange(points: CurvePoint[]): CurveAxisRange {
  const temps = points.map(([temp]) => temp);
  const lowest = Math.min(CURVE_TEMP_MIN, ...temps);
  const highest = Math.max(CURVE_TEMP_MAX, ...temps);
  return {
    min: Math.max(CURVE_POINT_TEMP_MIN, Math.floor(lowest / CURVE_AXIS_STEP) * CURVE_AXIS_STEP),
    max: Math.min(CURVE_POINT_TEMP_MAX, Math.ceil(highest / CURVE_AXIS_STEP) * CURVE_AXIS_STEP),
  };
}

export function canInsertCurvePoint(points: CurvePoint[]): boolean {
  return points.length < MAX_CURVE_POINTS;
}

/** Adds a point in the middle of the widest temperature gap on the chart, on the existing line. */
export function insertCurvePoint(points: CurvePoint[]): CurvePoint[] {
  if (!canInsertCurvePoint(points)) return points;
  const sorted = sortCurve(points);
  const range = curveAxisRange(sorted);
  // A hand-edited config can hold points past the axis; only gaps on the chart count.
  const visible = sorted.map(([temp]) => temp).filter((temp) => temp >= range.min && temp <= range.max);
  const bounds = [range.min, ...visible, range.max];
  let gapStart = range.min;
  let gapWidth = -1;
  for (let i = 1; i < bounds.length; i++) {
    const width = bounds[i] - bounds[i - 1];
    if (width > gapWidth) {
      gapWidth = width;
      gapStart = bounds[i - 1];
    }
  }
  const temp = Math.round((gapStart + gapWidth / 2) * 10) / 10;
  if (sorted.some(([existing]) => existing === temp)) return points;
  const speed = Math.round(speedAt(sorted, temp));
  return sortCurve([...sorted, [temp, speed]]);
}
