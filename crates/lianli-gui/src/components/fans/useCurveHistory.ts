import { computed, shallowRef } from "vue";
import type { FanCurve } from "@/types";
import type { CurvePoint } from "./curvePoints";

const MAX_STEPS = 50;
const MERGE_WINDOW_MS = 600;

interface Step {
  curve: FanCurve;
  name: string;
  before: CurvePoint[];
  after: CurvePoint[];
}

function copy(points: CurvePoint[]): CurvePoint[] {
  return points.map(([temp, speed]) => [temp, speed]);
}

/**
 * Undo and redo for curve points. Edits to the same curve that arrive within
 * MERGE_WINDOW_MS of each other form one step, so a drag undoes in one go.
 */
export function useCurveHistory(curves: () => FanCurve[], onRestore: () => void) {
  const undoStack = shallowRef<Step[]>([]);
  const redoStack = shallowRef<Step[]>([]);
  let lastEditAt = 0;

  function record(curve: FanCurve, before: CurvePoint[], after: CurvePoint[]) {
    const now = Date.now();
    const last = undoStack.value[undoStack.value.length - 1];
    if (last && last.curve === curve && now - lastEditAt < MERGE_WINDOW_MS) {
      undoStack.value = [...undoStack.value.slice(0, -1), { ...last, after: copy(after) }];
    } else {
      undoStack.value = [...undoStack.value, { curve, name: curve.name, before: copy(before), after: copy(after) }].slice(-MAX_STEPS);
    }
    redoStack.value = [];
    lastEditAt = now;
  }

  function move(from: typeof undoStack, to: typeof undoStack, pick: (step: Step) => CurvePoint[]) {
    while (from.value.length) {
      const step = from.value[from.value.length - 1];
      from.value = from.value.slice(0, -1);
      const list = curves();
      const target = list.find((curve) => curve === step.curve) ?? list.find((curve) => curve.name === step.name);
      if (!target) continue;
      target.curve = copy(pick(step));
      to.value = [...to.value, { ...step, curve: target, name: target.name }];
      lastEditAt = 0;
      onRestore();
      return;
    }
  }

  return {
    canUndo: computed(() => undoStack.value.length > 0),
    canRedo: computed(() => redoStack.value.length > 0),
    record,
    undo: () => move(undoStack, redoStack, (step) => step.before),
    redo: () => move(redoStack, undoStack, (step) => step.after),
  };
}
