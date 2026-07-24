import { onScopeDispose, ref, type Ref } from "vue";

type AsyncTask = () => Promise<void>;

/**
 * Reusable 2s polling loop.
 *
 * Runs `task` immediately, then every `intervalMs`. Pauses while a previous
 * tick is still in flight to avoid overlapping daemon round-trips. Disposes
 * the timer when the owning component/store scope is destroyed.
 */
export function usePolling(task: AsyncTask, intervalMs = 2000): {
  running: Ref<boolean>;
  start: () => void;
  stop: () => void;
  tick: () => Promise<void>;
} {
  const running = ref(false);
  let timer: ReturnType<typeof setInterval> | null = null;
  let inFlight = false;

  async function tick() {
    if (inFlight) return;
    inFlight = true;
    try {
      await task();
    } finally {
      inFlight = false;
    }
  }

  function start() {
    if (running.value) return;
    running.value = true;
    void tick();
    timer = setInterval(() => void tick(), intervalMs);
  }

  function stop() {
    running.value = false;
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  onScopeDispose(stop);

  return { running, start, stop, tick };
}
