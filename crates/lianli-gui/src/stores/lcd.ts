import { defineStore } from "pinia";
import { ref } from "vue";
import { emit } from "@tauri-apps/api/event";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import type { LcdConfig, LcdTemplate } from "@/types";

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

  async function setBrightness(deviceId: string, brightness: number) {
    await ipc.request("SetLcdBrightness", { device_id: deviceId, brightness });
  }

  /**
   * Request a JPEG preview render from the daemon (debounced 200ms) so the
   * editor canvas updates live without flooding the daemon on every property
   * edit.
   */
  function renderPreview() {
    const run = useDebounce(async (template: LcdTemplate, w: number, h: number) => {
      previewLoading.value = true;
      try {
        const res = await ipc.request<{ jpeg_base64: string }>("RenderTemplatePreview", {
          template,
          width: w,
          height: h,
        });
        previewJpeg.value = res.jpeg_base64;
      } finally {
        previewLoading.value = false;
      }
    }, 200);
    return run;
  }

  return {
    previewJpeg,
    previewLoading,
    switchDisplayMode,
    setLcdMedia,
    setTemplates,
    setBrightness,
    renderPreview,
  };
});
