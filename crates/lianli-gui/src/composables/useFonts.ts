import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface SystemFont {
  family: string;
  path: string;
}

const fonts = ref<SystemFont[]>([]);
let loaded = false;
let loading: Promise<void> | null = null;

/**
 * Lazily fetch the system font list (via the Rust `list_system_fonts`
 * command, which shells out to `fc-list`). Cached for the app lifetime.
 */
export function useFonts() {
  async function load() {
    if (loaded) return;
    if (loading) return loading;
    loading = (async () => {
      try {
        fonts.value = await invoke<SystemFont[]>("list_system_fonts");
        loaded = true;
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn("list_system_fonts failed", e);
      } finally {
        loading = null;
      }
    })();
    return loading;
  }

  /** Build NSelect options: a "(Default)" sentinel + one entry per family. */
  function fontOptions(): { label: string; value: string }[] {
    return [
      { label: "(Default)", value: "" },
      ...fonts.value.map((f) => ({ label: f.family, value: f.path })),
    ];
  }

  return { fonts, load, fontOptions };
}
