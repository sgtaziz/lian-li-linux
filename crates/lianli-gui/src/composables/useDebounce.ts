import { onScopeDispose } from "vue";

/**
 * Returns a debounced wrapper around `fn`. Multiple calls within `ms`
 * collapse into a single trailing invocation with the latest arguments.
 *
 * Used for: 50ms RGB effect changes, 400ms fan-quantity steppers,
 * 200ms template preview rendering.
 */
export function useDebounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): { (...args: A): void; flush: () => void; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastArgs: A | null = null;

  function trigger(...args: A) {
    lastArgs = args;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      if (lastArgs) {
        const a = lastArgs;
        lastArgs = null;
        fn(...a);
      }
    }, ms);
  }

  trigger.flush = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (lastArgs) {
      const a = lastArgs;
      lastArgs = null;
      fn(...a);
    }
  };

  trigger.cancel = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    lastArgs = null;
  };

  onScopeDispose(() => {
    if (timer !== null) clearTimeout(timer);
  });

  return trigger;
}
