import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@/utils/backendText";
import { applyPalette, buildPalette, DEFAULT_ACCENT, isHexColor, naiveOverrides } from "@/theme/palette";

export type ThemeMode = "dark" | "light";
export type ThemePreference = ThemeMode | "system";
export type AccentMode = "system" | "custom";

interface DesktopAppearance {
  accent: string | null;
  color_scheme: ThemeMode | null;
}

type AppearanceChange =
  | { key: "accent"; value: string | null }
  | { key: "color_scheme"; value: ThemeMode | null };

const THEME_KEY = "lianli-linux.theme";
const ACCENT_MODE_KEY = "lianli-linux.accentMode";
const CUSTOM_ACCENT_KEY = "lianli-linux.customAccent";
const APPEARANCE_EVENT = "desktop-appearance-changed";

function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStored(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Display preferences stay in memory when storage is unavailable.
  }
}

function browserScheme(): ThemeMode {
  try {
    return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export const useThemeStore = defineStore("theme", () => {
  const storedTheme = readStored(THEME_KEY);
  const preference = ref<ThemePreference>(
    storedTheme === "dark" || storedTheme === "light" ? storedTheme : "system",
  );
  const accentMode = ref<AccentMode>(readStored(ACCENT_MODE_KEY) === "custom" ? "custom" : "system");
  const storedCustom = readStored(CUSTOM_ACCENT_KEY);
  const customAccent = ref(isHexColor(storedCustom) ? storedCustom : DEFAULT_ACCENT);

  const desktopAccent = ref<string | null>(null);
  const desktopScheme = ref<ThemeMode | null>(null);
  const fallbackScheme = ref<ThemeMode>(browserScheme());

  const mode = computed<ThemeMode>(() =>
    preference.value === "system" ? (desktopScheme.value ?? fallbackScheme.value) : preference.value,
  );
  const isDark = computed(() => mode.value === "dark");
  const accent = computed(() =>
    accentMode.value === "custom" ? customAccent.value : (desktopAccent.value ?? DEFAULT_ACCENT),
  );
  const palette = computed(() => buildPalette(accent.value, isDark.value));
  const overrides = computed(() => naiveOverrides(palette.value));

  function set(next: ThemeMode) {
    preference.value = next;
  }

  function toggle() {
    preference.value = isDark.value ? "light" : "dark";
  }

  function setPreference(next: ThemePreference) {
    preference.value = next;
  }

  function setAccentMode(next: AccentMode) {
    accentMode.value = next;
  }

  function setCustomAccent(hex: string) {
    if (!isHexColor(hex)) return;
    customAccent.value = hex.toLowerCase();
    accentMode.value = "custom";
  }

  function applyChange(change: AppearanceChange) {
    if (change.key === "accent") desktopAccent.value = isHexColor(change.value) ? change.value : null;
    else desktopScheme.value = change.value;
  }

  let started = false;
  async function start() {
    if (started) return;
    started = true;
    try {
      window.matchMedia?.("(prefers-color-scheme: light)").addEventListener("change", (event) => {
        fallbackScheme.value = event.matches ? "light" : "dark";
      });
    } catch {
      // Without matchMedia the dark fallback stays in place.
    }
    try {
      await listen<AppearanceChange>(APPEARANCE_EVENT, (event) => applyChange(event.payload));
      const appearance = await invoke<DesktopAppearance>("desktop_appearance");
      desktopAccent.value = isHexColor(appearance.accent) ? appearance.accent : null;
      desktopScheme.value = appearance.color_scheme;
    } catch {
      // Outside Tauri, or without a desktop portal, the defaults stay in place.
    }
  }

  watch(preference, (next) => writeStored(THEME_KEY, next));
  watch(accentMode, (next) => writeStored(ACCENT_MODE_KEY, next));
  watch(customAccent, (next) => writeStored(CUSTOM_ACCENT_KEY, next));
  watch(
    [mode, palette],
    ([nextMode, nextPalette]) => {
      const root = document.documentElement;
      root.setAttribute("data-theme", nextMode);
      root.style.colorScheme = nextMode;
      applyPalette(nextPalette, root);
    },
    { immediate: true },
  );

  return {
    preference,
    mode,
    isDark,
    desktopScheme,
    accentMode,
    customAccent,
    desktopAccent,
    accent,
    palette,
    overrides,
    set,
    toggle,
    setPreference,
    setAccentMode,
    setCustomAccent,
    start,
  };
});
