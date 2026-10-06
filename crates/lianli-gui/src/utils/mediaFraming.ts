import type { MediaFraming } from "@/types";

export const MAX_ZOOM = 8;

export function defaultFraming(): MediaFraming {
  return { fit: "stretch", zoom: 1, offset_x: 0, offset_y: 0 };
}

export function isDefaultFraming(framing: MediaFraming | undefined): boolean {
  if (!framing) return true;
  return framing.fit === "stretch" && framing.zoom === 1 && framing.offset_x === 0 && framing.offset_y === 0;
}

export function clampFraming(framing: MediaFraming): MediaFraming {
  const finite = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback);
  return {
    fit: framing.fit,
    zoom: Math.min(MAX_ZOOM, Math.max(1, finite(framing.zoom, 1))),
    offset_x: Math.min(1, Math.max(-1, finite(framing.offset_x, 0))),
    offset_y: Math.min(1, Math.max(-1, finite(framing.offset_y, 0))),
  };
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Mirrors `MediaFraming::source_crop` in lianli-shared. */
export function sourceCrop(framing: MediaFraming, source: [number, number], target: [number, number]): Rect {
  const f = clampFraming(framing);
  const [sw, sh] = [Math.max(1, source[0]), Math.max(1, source[1])];
  const [tw, th] = [Math.max(1, target[0]), Math.max(1, target[1])];
  const [baseW, baseH] = f.fit === "cover" ? [Math.min(sw, (sh * tw) / th), Math.min(sh, (sw * th) / tw)] : [sw, sh];
  const width = Math.min(sw, Math.max(1, Math.round(baseW / f.zoom)));
  const height = Math.min(sh, Math.max(1, Math.round(baseH / f.zoom)));
  const x = Math.min(sw - width, Math.max(0, Math.round(((sw - width) / 2) * (1 + f.offset_x))));
  const y = Math.min(sh - height, Math.max(0, Math.round(((sh - height) / 2) * (1 + f.offset_y))));
  return { x, y, width, height };
}

/**
 * CSS placement, in percent of the screen box, that reproduces the LCD output:
 * `box` is where the selected region lands and `media` positions the whole
 * source inside that box.
 */
export function previewPlacement(framing: MediaFraming, source: [number, number], target: [number, number]) {
  const crop = sourceCrop(framing, source, target);
  let box: Rect = { x: 0, y: 0, width: 100, height: 100 };
  if (framing.fit === "contain") {
    const scale = Math.min(target[0] / crop.width, target[1] / crop.height);
    const width = ((crop.width * scale) / target[0]) * 100;
    const height = ((crop.height * scale) / target[1]) * 100;
    box = { x: (100 - width) / 2, y: (100 - height) / 2, width, height };
  }
  const media: Rect = {
    x: (-crop.x / crop.width) * 100,
    y: (-crop.y / crop.height) * 100,
    width: (source[0] / crop.width) * 100,
    height: (source[1] / crop.height) * 100,
  };
  return { crop, box, media };
}

/** Offset change for dragging the preview by a fraction of the box size. */
export function panBy(
  framing: MediaFraming,
  source: [number, number],
  target: [number, number],
  dxFraction: number,
  dyFraction: number,
): MediaFraming {
  const crop = sourceCrop(framing, source, target);
  const slackX = (source[0] - crop.width) / 2;
  const slackY = (source[1] - crop.height) / 2;
  return clampFraming({
    ...framing,
    offset_x: slackX > 0 ? framing.offset_x - (dxFraction * crop.width) / slackX : framing.offset_x,
    offset_y: slackY > 0 ? framing.offset_y - (dyFraction * crop.height) / slackY : framing.offset_y,
  });
}
