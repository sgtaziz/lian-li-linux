import type { MessageKey } from "@/i18n";
import type { CurvePoint } from "./curvePoints";

export interface CurveTemplate {
  id: string;
  nameKey: MessageKey;
  points: CurvePoint[];
}

export const CURVE_TEMPLATES: CurveTemplate[] = [
  { id: "quiet", nameKey: "fans.templates.quiet", points: [[30, 20], [50, 25], [65, 40], [75, 65], [85, 100]] },
  { id: "standard", nameKey: "fans.templates.standard", points: [[30, 30], [50, 40], [65, 60], [75, 80], [85, 100]] },
  { id: "high-speed", nameKey: "fans.templates.highSpeed", points: [[30, 45], [50, 60], [60, 75], [70, 90], [80, 100]] },
  { id: "full-speed", nameKey: "fans.templates.fullSpeed", points: [[20, 100], [100, 100]] },
];
