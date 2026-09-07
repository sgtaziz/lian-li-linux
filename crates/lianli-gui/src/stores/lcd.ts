import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { emit } from "@tauri-apps/api/event";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import type { CatalogTemplate, LcdConfig, LcdTemplate } from "@/types";

/** Broadcast when SetLcdTemplates changes the template list, so other open
 *  windows (each with their own config store instance) know to reload it. */
export const LCD_TEMPLATES_CHANGED_EVENT = "lcd-templates-changed";

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

  const cleaningActive = ref(false);
  const cleaningDeviceId = ref<string | null>(null);
  const cleaningDurationMinutes = ref(30);
  const remainingSeconds = ref(0);
  let timerInterval: ReturnType<typeof setInterval> | null = null;

  const TIMER_TICK_INTERVAL_MS = 1000; // Timer tick interval in milliseconds (1 second)

  // Formats remaining countdown into human-readable "Xm YYs" (e.g., "29m 05s"):
  // `m` represents remaining whole minutes, and `s` is remaining seconds (0-59, zero-padded).
  const formattedRemaining = computed(() => {
    const secs = remainingSeconds.value;
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return `${m}m ${s < 10 ? "0" : ""}${s}s`;
  });

  function startTimer(durationMinutes: number) {
    if (timerInterval) {
      clearInterval(timerInterval);
      timerInterval = null;
    }
    const totalSecs = durationMinutes * 60;
    remainingSeconds.value = totalSecs;
    const endsAt = Date.now() + totalSecs * 1000;

    timerInterval = setInterval(() => {
      // `left` is the remaining seconds until target completion timestamp (`endsAt`),
      // calculated from epoch time to avoid cumulative drift caused by timer throttling.
      const left = Math.max(0, Math.round((endsAt - Date.now()) / 1000));
      remainingSeconds.value = left;
      if (left <= 0) {
        cleaningActive.value = false;
        cleaningDeviceId.value = null;
        if (timerInterval) {
          clearInterval(timerInterval);
          timerInterval = null;
        }
      }
    }, TIMER_TICK_INTERVAL_MS);
  }

  function stopTimer() {
    if (timerInterval) {
      clearInterval(timerInterval);
      timerInterval = null;
    }
    remainingSeconds.value = 0;
  }

  async function startPixelClean(
    deviceId?: string | null,
    durationMinutes: number = 30,
  ) {
    const res = await ipc.request("StartPixelClean", {
      device_id: deviceId ?? null,
      duration_minutes: durationMinutes,
    });
    cleaningActive.value = true;
    cleaningDeviceId.value = deviceId ?? null;
    cleaningDurationMinutes.value = durationMinutes;
    startTimer(durationMinutes);
    return res;
  }

  async function stopPixelClean(deviceId?: string | null) {
    const res = await ipc.request("StopPixelClean", {
      device_id: deviceId ?? null,
    });
    cleaningActive.value = false;
    cleaningDeviceId.value = null;
    stopTimer();
    return res;
  }

  return {
    previewJpeg,
    previewLoading,
    cleaningActive,
    cleaningDeviceId,
    cleaningDurationMinutes,
    remainingSeconds,
    formattedRemaining,
    switchDisplayMode,
    setLcdMedia,
    setTemplates,
    installTemplate,
    setBrightness,
    renderPreview,
    startPixelClean,
    stopPixelClean,
  };
});
