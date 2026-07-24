import { defineStore } from "pinia";
import { ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { useDebounce } from "@/composables/useDebounce";
import type { FanSpeed } from "@/types";

/**
 * Fan-side effects. The ENE6K77 fan-quantity stepper is debounced 400ms;
 * everything else is sent immediately. Fan speed/PWM changes go through
 * config save (SetConfig) rather than live IPC, matching the Slint GUI.
 */
export const useFansStore = defineStore("fans", () => {
  const ipc = useIpc();

  // Debounced fan-quantity overrides keyed by device_id.
  const pendingQuantities = ref<Map<string, number>>(new Map());

  /**
   * Debounce a fan-quantity change by 400ms. Rapid stepper clicks collapse
   * into the final value, then a single SetEne6k77FanQuantity is sent.
   */
  function scheduleFanQuantity(
    deviceId: string,
    quantity: number,
    onDone?: () => void,
  ) {
    pendingQuantities.value.set(deviceId, quantity);
    const flush = useDebounce(() => {
      const q = pendingQuantities.value.get(deviceId);
      if (q === undefined) return;
      pendingQuantities.value.delete(deviceId);
      void ipc
        .request("SetEne6k77FanQuantity", { device_id: deviceId, quantity: q })
        .finally(() => onDone?.());
    }, 400);
    flush();
  }

  function fanSpeedLabel(speed: FanSpeed, curves: string[]): string {
    if (typeof speed === "number") return `PWM ${Math.round((speed / 255) * 100)}%`;
    if (speed === "__mb_sync__") return "MB Sync";
    if (speed.startsWith("__mb_sync__:")) return `MB Sync (${speed.slice("__mb_sync__:".length)})`;
    if (speed === "off" || speed === "") return "Off";
    if (curves.includes(speed)) return `Curve: ${speed}`;
    return speed;
  }

  return {
    pendingQuantities,
    scheduleFanQuantity,
    fanSpeedLabel,
  };
});
