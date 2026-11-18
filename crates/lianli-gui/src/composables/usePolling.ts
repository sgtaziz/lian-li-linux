import { onScopeDispose, ref, type Ref } from "vue";

type AsyncTask = () => Promise<void>;

/**
 * Reusable polling loop.
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
  setIntervalMs: (ms: number) => void;
} {
  const running = ref(false);
  let timer: ReturnType<typeof setInterval> | null = null;
  let inFlight: Promise<void> | null = null;

  function tick(): Promise<void> {
    if (inFlight) return inFlight;
    inFlight = Promise.resolve().then(task).finally(() => {
      inFlight = null;
    });
    return inFlight;
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

  function setIntervalMs(ms: number) {
    if (ms === intervalMs) return;
    intervalMs = ms;
    if (running.value) {
      stop();
      start();
    }
  }

  onScopeDispose(stop);

  return { running, start, stop, tick, setIntervalMs };
}
