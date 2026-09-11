import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { emit } from "@tauri-apps/api/event";
import { useIpc } from "@/composables/useIpc";
import { PIXEL_CLEANER_DURATION_OPTIONS } from "@/constants";
import { useDebounce } from "@/composables/useDebounce";
import type { CatalogTemplate, LcdConfig, LcdTemplate, PixelCleanStatus } from "@/types";

/** Broadcast when SetLcdTemplates changes the template list, so other open
 *  windows (each with their own config store instance) know to reload it. */
export const LCD_TEMPLATES_CHANGED_EVENT = "lcd-templates-changed";

export interface ActiveCleanerSession {
  sessionId?: number | null;
  deviceId?: string | null;
  durationMinutes: number;
  remainingSeconds: number;
  endsAt: number;
}

const MS_PER_SECOND = 1000;
const SECONDS_PER_MINUTE = 60;
/** Maximum timer drift in seconds allowed between client timer and daemon telemetry before re-anchoring completion timestamp. */
const MAX_TIMER_DRIFT_SECONDS = 2;

function hidIdNorm(id: string): string {
  return id.replace(/^hidraw:/, "").replace(/^hid:/, "");
}

function formatSeconds(secs: number): string {
  const m = Math.floor(secs / SECONDS_PER_MINUTE);
  const s = secs % SECONDS_PER_MINUTE;
  return `${m}m ${s < 10 ? "0" : ""}${s}s`;
}

function cleanerMatchesTarget(key: string, targetId?: string | null, cardIndex?: number): boolean {
  if (key === "all") return true;
  if (targetId && key === targetId) return true;
  if (cardIndex !== undefined && (key === `${cardIndex}` || key === `index:${cardIndex}` || key === `lcd:${cardIndex}`)) return true;
  if (targetId) {
    const keyBase = key.split("#")[0];
    const targetBase = targetId.split("#")[0];
    if (keyBase && targetBase && (keyBase === targetBase || hidIdNorm(keyBase) === hidIdNorm(targetBase))) {
      if (key.includes("#") && targetId.includes("#")) {
        return key.split("#")[1] === targetId.split("#")[1];
      }
      return true;
    }
  }
  return false;
}

/**
 * LCD-side effects: display-mode switch, media apply, and the template editor
 * preview renderer (debounced 200ms). Template persistence goes through
 * SetLcdTemplates.
 */
export const useLcdStore = defineStore("lcd", () => {
  const ipc = useIpc();

  // Last preview JPEG (base64) keyed by an arbitrary request id.
  const previewJpeg = ref<string>("");
  const previewLoading = ref(false);

  async function switchDisplayMode(deviceId: string) {
    await ipc.request("SwitchDisplayMode", { device_id: deviceId });
  }

  async function setLcdMedia(deviceId: string, cfg: LcdConfig) {
    await ipc.request("SetLcdMedia", { device_id: deviceId, config: cfg });
  }

  async function setTemplates(templates: LcdTemplate[]) {
    await ipc.request("SetLcdTemplates", { templates });
    await emit(LCD_TEMPLATES_CHANGED_EVENT);
  }

  async function installTemplate(template: CatalogTemplate) {
    await ipc.request("InstallTemplate", { template });
    await emit(LCD_TEMPLATES_CHANGED_EVENT);
  }

  async function setBrightness(deviceId: string, brightness: number) {
    if (
      import.meta.env.DEV &&
      (deviceId.includes("mock") ||
        (typeof window !== "undefined" && !(window as any).__TAURI_INTERNALS__))
    ) {
      return;
    }
    await ipc.request("SetLcdBrightness", { device_id: deviceId, brightness });
  }

  /**
   * Request a JPEG preview render from the daemon (debounced 200ms) so the
   * editor canvas updates live without flooding the daemon on every property
   * edit.
   */
  function renderPreview() {
    const run = useDebounce(
      async (template: LcdTemplate, w: number, h: number) => {
        previewLoading.value = true;
        try {
          const res = await ipc.request<{ jpeg_base64: string }>(
            "RenderTemplatePreview",
            {
              template,
              width: w,
              height: h,
            },
          );
          previewJpeg.value = res.jpeg_base64;
        } finally {
          previewLoading.value = false;
        }
      },
      200,
    );
    return run;
  }

  const activeCleaners = ref<Record<string, ActiveCleanerSession>>({});
  const cleaningActive = computed(() => Object.keys(activeCleaners.value).length > 0);

  // Backward compatibility getters for first active cleaner
  const firstActiveCleaner = computed<ActiveCleanerSession | undefined>(() => {
    return Object.values(activeCleaners.value)[0];
  });
  const cleaningDeviceId = computed(() => firstActiveCleaner.value?.deviceId ?? null);
  const cleaningDurationMinutes = computed(() => firstActiveCleaner.value?.durationMinutes ?? 30);
  const remainingSeconds = computed(() => firstActiveCleaner.value?.remainingSeconds ?? 0);
  const currentSessionId = computed(() => firstActiveCleaner.value?.sessionId ?? null);
  const formattedRemaining = computed(() => {
    return firstActiveCleaner.value ? formatSeconds(firstActiveCleaner.value.remainingSeconds) : "0m 00s";
  });

  function isCleaning(targetId?: string | null, cardIndex?: number): boolean {
    if (activeCleaners.value["all"]) return true;
    return Object.keys(activeCleaners.value).some((key) => cleanerMatchesTarget(key, targetId, cardIndex));
  }

  function formattedRemainingFor(targetId?: string | null, cardIndex?: number): string {
    if (activeCleaners.value["all"]) {
      return formatSeconds(activeCleaners.value["all"].remainingSeconds);
    }
    for (const [key, session] of Object.entries(activeCleaners.value)) {
      if (cleanerMatchesTarget(key, targetId, cardIndex)) {
        return formatSeconds(session.remainingSeconds);
      }
    }
    return "0m 00s";
  }

  function getSession(targetId?: string | null, cardIndex?: number): ActiveCleanerSession | undefined {
    if (targetId && activeCleaners.value[targetId]) return activeCleaners.value[targetId];
    if (activeCleaners.value["all"]) return activeCleaners.value["all"];
    for (const [key, session] of Object.entries(activeCleaners.value)) {
      if (cleanerMatchesTarget(key, targetId, cardIndex)) {
        return session;
      }
    }
    return undefined;
  }

  let timerInterval: ReturnType<typeof setInterval> | null = null;
  const TIMER_TICK_INTERVAL_MS = MS_PER_SECOND;

  function ensureTimerRunning() {
    if (timerInterval) return;
    timerInterval = setInterval(() => {
      const now = Date.now();
      let hasAny = false;
      const next: Record<string, ActiveCleanerSession> = {};
      for (const [key, session] of Object.entries(activeCleaners.value)) {
        const left = Math.max(0, Math.round((session.endsAt - now) / MS_PER_SECOND));
        next[key] = { ...session, remainingSeconds: left };
        hasAny = true;
      }
      activeCleaners.value = next;
      if (!hasAny) {
        stopTimer();
      }
    }, TIMER_TICK_INTERVAL_MS);
  }

  function stopTimer() {
    if (timerInterval) {
      clearInterval(timerInterval);
      timerInterval = null;
    }
  }

  function applyCleanerTelemetry(
    statuses?: Record<string, PixelCleanStatus> | null,
  ) {
    if (statuses && Object.keys(statuses).length > 0) {
      const now = Date.now();
      const updated: Record<string, ActiveCleanerSession> = {};
      for (const [key, s] of Object.entries(statuses)) {
        if (s.active) {
          const existing = activeCleaners.value[key];
          const hasDrifted =
            !existing || existing.sessionId !== s.session_id ||
            Math.abs(existing.remainingSeconds - s.remaining_seconds) > MAX_TIMER_DRIFT_SECONDS;
          const endsAt = hasDrifted
            ? now + s.remaining_seconds * MS_PER_SECOND
            : existing.endsAt;
          updated[key] = {
            sessionId: s.session_id,
            deviceId: s.device_id,
            durationMinutes: s.duration_minutes,
            remainingSeconds: s.remaining_seconds,
            endsAt,
          };
        }
      }
      activeCleaners.value = updated;
      if (Object.keys(updated).length > 0) {
        ensureTimerRunning();
      } else {
        stopTimer();
      }
    } else {
      activeCleaners.value = {};
      stopTimer();
    }
  }

  const pendingCleaners = ref<Record<string, { sessionId: number | null; cancelled: boolean }>>({});
  const preparingCleaner = computed(() => Object.keys(pendingCleaners.value).length > 0);

  function isPreparing(targetId: string): boolean {
    return !!pendingCleaners.value[targetId];
  }

  function cancelPixelPreparation(targetId: string) {
    const pending = pendingCleaners.value[targetId];
    if (pending) pending.cancelled = true;
  }

  async function refreshCleanerStatus() {
    applyCleanerTelemetry(await ipc.request<Record<string, PixelCleanStatus>>("GetPixelCleanStatus"));
  }

  async function startPixelClean(deviceId?: string | null, durationMinutes: number = 30) {
    if (!PIXEL_CLEANER_DURATION_OPTIONS.some((option) => option.value === durationMinutes)) {
      throw new Error("Invalid pixel cleaner duration");
    }
    const key = deviceId ?? "all";
    if (preparingCleaner.value) throw new Error("Another cleaner is being prepared");
    const pending = { sessionId: null as number | null, cancelled: false };
    pendingCleaners.value[key] = pending;
    let activated = false;
    try {
      let response = await ipc.request<{ started: boolean; session_id: number }>("StartPixelClean", {
        device_id: deviceId ?? null, duration_minutes: durationMinutes, preparation_id: null,
      });
      if (!Number.isSafeInteger(response.session_id)) throw new Error("Daemon did not return a session ID");
      pending.sessionId = response.session_id;
      activated = response.started === true;
      const deadline = Date.now() + 90_000;
      while (!activated && !pending.cancelled) {
        if (Date.now() >= deadline) throw new Error("Pixel cleaner preparation timed out");
        const status = await ipc.request<{ ready: boolean; error: string | null }>("GetPixelCleanPreparation", {
          session_id: pending.sessionId,
        });
        if (status.error) throw new Error(status.error);
        if (pending.cancelled) break;
        if (status.ready) {
          response = await ipc.request("StartPixelClean", {
            device_id: deviceId ?? null, duration_minutes: durationMinutes, preparation_id: pending.sessionId,
          });
          if (response.started !== true) throw new Error("Daemon did not confirm activation");
          activated = true;
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      if (pending.cancelled) {
        activated = false;
        return { started: false, cancelled: true };
      }
      await refreshCleanerStatus();
      return { started: true, cancelled: false };
    } finally {
      if (!activated && pending.sessionId !== null) {
        // Unconfirmed preparations cannot activate and expire in the daemon.
        await ipc.request("StopPixelClean", { device_id: null, session_id: pending.sessionId }).catch(() => {});
      }
      delete pendingCleaners.value[key];
    }
  }

  async function stopPixelClean(deviceId?: string | null) {
    const session = getSession(deviceId);
    if (session?.sessionId == null) {
      await refreshCleanerStatus();
      return { stopped: false };
    }
    const result = await ipc.request<{ stopped: boolean }>("StopPixelClean", {
      device_id: deviceId ?? null, session_id: session.sessionId,
    });
    await refreshCleanerStatus();
    return result;
  }

  return {
    previewJpeg,
    previewLoading,
    activeCleaners,
    cleaningActive,
    cleaningDeviceId,
    cleaningDurationMinutes,
    currentSessionId,
    remainingSeconds,
    formattedRemaining,
    isCleaning,
    formattedRemainingFor,
    switchDisplayMode,
    setLcdMedia,
    setTemplates,
    installTemplate,
    setBrightness,
    renderPreview,
    startPixelClean,
    preparingCleaner,
    isPreparing,
    cancelPixelPreparation,
    stopPixelClean,
    applyCleanerTelemetry,
  };
});
