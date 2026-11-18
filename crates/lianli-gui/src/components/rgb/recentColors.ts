import { ref } from "vue";

const STORAGE_KEY = "lianli-linux.recentColors";
const MAX_RECENT = 8;

export const PRESET_COLORS = [
  "#ff0000",
  "#ff6a00",
  "#ffd400",
  "#00ff40",
  "#00e5ff",
  "#0050ff",
  "#8a2be2",
  "#ffffff",
];

export const recentColors = ref<string[]>(load());

function load(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((value): value is string => typeof value === "string" && /^#[0-9a-f]{6}$/.test(value)).slice(0, MAX_RECENT);
  } catch {
    return [];
  }
}

export function rememberColor(hex: string) {
  const color = hex.toLowerCase();
  if (!/^#[0-9a-f]{6}$/.test(color)) return;
  recentColors.value = [color, ...recentColors.value.filter((value) => value !== color)].slice(0, MAX_RECENT);
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(recentColors.value));
  } catch {
    // Recent colours are a convenience; storage may be unavailable.
  }
}
