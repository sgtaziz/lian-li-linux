import { defineStore } from "pinia";
import { ref } from "vue";

const STORAGE_KEY = "lianli.lcdNames";
const MAX_NAME_LENGTH = 40;

function readStored(): Record<string, string> {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
    if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
    return Object.fromEntries(
      Object.entries(stored).filter((entry): entry is [string, string] => typeof entry[1] === "string"),
    );
  } catch {
    return {};
  }
}

/** User-chosen screen names, keyed by `lcdEntryKey`. A display preference only. */
export const useLcdNamesStore = defineStore("lcdNames", () => {
  const names = ref<Record<string, string>>(readStored());

  function setName(key: string, name: string) {
    const trimmed = name.trim().slice(0, MAX_NAME_LENGTH);
    const next = { ...names.value };
    if (trimmed) next[key] = trimmed;
    else delete next[key];
    names.value = next;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Names stay for this session when storage is unavailable.
    }
  }

  return { names, setName };
});
