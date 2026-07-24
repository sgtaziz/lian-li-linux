import { defineStore } from "pinia";
import { ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import { useConfigStore } from "@/stores/config";
import type { RgbEffect, RGB } from "@/types";

/**
 * RGB-specific side effects: SetRgbEffect (debounced 50ms), SetRgbDirect,
 * SetLedColor, SetMbRgbSync, SetFanDirection, and preset CRUD.
 *
 * All mutations also update the config mirror in the config store so a Save
 * persists them; live changes additionally fire an IPC command immediately.
 */
export const useRgbStore = defineStore("rgb", () => {
  const ipc = useIpc();
  const config = useConfigStore();

  // Pending (debounced) effect requests keyed by `${deviceId}:${zone}`.
  const pendingEffects = ref<Map<string, () => void>>(new Map());

  /** Send a single SetRgbEffect now. */
  async function sendEffect(deviceId: string, zone: number, effect: RgbEffect) {
    await ipc.request("SetRgbEffect", { device_id: deviceId, zone, effect });
  }

  /**
   * Debounce an effect change by 50ms. Multiple rapid changes for the same
   * device+zone collapse into the latest value.
   */
  function scheduleEffect(deviceId: string, zone: number, effect: RgbEffect) {
    const key = `${deviceId}:${zone}`;
    const send = () => {
      pendingEffects.value.delete(key);
      void sendEffect(deviceId, zone, effect);
    };
    pendingEffects.value.set(key, send);
    const flush = useDebounce(() => {
      const fn = pendingEffects.value.get(key);
      if (fn) fn();
    }, 50);
    flush();
  }

  /** Send the full per-zone direct LED buffer. */
  async function sendDirect(deviceId: string, zone: number, colors: RGB[]) {
    await ipc.request("SetRgbDirect", { device_id: deviceId, zone, colors });
  }

  /** Set a single LED by index. */
  async function setLedColor(deviceId: string, zone: number, ledIndex: number, color: RGB) {
    await ipc.request("SetLedColor", { device_id: deviceId, zone, led_index: ledIndex, color });
  }

  /** Fetch the live per-LED colors for a zone (wireless devices). */
  async function getZoneColors(deviceId: string, zone: number): Promise<RGB[]> {
    return ipc.request<RGB[]>("GetZoneColors", { device_id: deviceId, zone });
  }

  async function setMbSync(deviceId: string, enabled: boolean) {
    await ipc.request("SetMbRgbSync", { device_id: deviceId, enabled });
  }

  async function setFanDirection(
    deviceId: string,
    zone: number,
    swapLr: boolean,
    swapTb: boolean,
  ) {
    await ipc.request("SetFanDirection", {
      device_id: deviceId,
      zone,
      swap_lr: swapLr,
      swap_tb: swapTb,
    });
  }

  // ── Presets ────────────────────────────────────────────────────────────────
  async function savePreset(name: string, deviceId: string) {
    await ipc.request("SaveRgbPreset", { name, device_id: deviceId });
    await config.load();
  }

  async function applyPreset(name: string, deviceId: string) {
    await ipc.request("ApplyRgbPreset", { name, device_id: deviceId });
  }

  async function deletePreset(name: string, deviceId: string) {
    await ipc.request("DeleteRgbPreset", { name, device_id: deviceId });
    await config.load();
  }

  /** Flush every pending effect immediately (called before config save). */
  function flushPending() {
    for (const fn of pendingEffects.value.values()) fn();
    pendingEffects.value.clear();
  }

  config.registerFlush(flushPending);

  return {
    pendingEffects,
    sendEffect,
    scheduleEffect,
    sendDirect,
    setLedColor,
    getZoneColors,
    setMbSync,
    setFanDirection,
    savePreset,
    applyPreset,
    deletePreset,
    flushPending,
  };
});
