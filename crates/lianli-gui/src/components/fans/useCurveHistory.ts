import { computed, shallowRef, watch } from "vue";
import type { FanCurve } from "@/types";
import type { CurvePoint } from "./curvePoints";

const MAX_STEPS = 50;

let lastGesture = 0;

/** Identifies one press-to-release edit, so all of its updates undo as one step. */
export function newCurveGesture(): number {
  lastGesture += 1;
  return lastGesture;
}

interface Step {
  curve: FanCurve;
  gesture: number | null;
  before: CurvePoint[];
  after: CurvePoint[];
}

function copy(points: CurvePoint[]): CurvePoint[] {
  return points.map(([temp, speed]) => [temp, speed]);
}

export function useCurveHistory(curves: () => FanCurve[], onRestore: () => void) {
  const undoStack = shallowRef<Step[]>([]);
  const redoStack = shallowRef<Step[]>([]);

  // A reloaded configuration holds new curve objects, so earlier steps no longer apply.
  watch(curves, () => {
    undoStack.value = [];
    redoStack.value = [];
  });

  function record(curve: FanCurve, before: CurvePoint[], after: CurvePoint[], gesture: number | null = null) {
    const last = undoStack.value[undoStack.value.length - 1];
    if (gesture !== null && last && last.curve === curve && last.gesture === gesture) {
      undoStack.value = [...undoStack.value.slice(0, -1), { ...last, after: copy(after) }];
    } else {
      undoStack.value = [...undoStack.value, { curve, gesture, before: copy(before), after: copy(after) }].slice(-MAX_STEPS);
    }
    redoStack.value = [];
  }

  function move(from: typeof undoStack, to: typeof undoStack, pick: (step: Step) => CurvePoint[]) {
    while (from.value.length) {
      const step = from.value[from.value.length - 1];
      from.value = from.value.slice(0, -1);
      if (!curves().includes(step.curve)) continue;
      step.curve.curve = copy(pick(step));
      to.value = [...to.value, { ...step, gesture: null }];
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
