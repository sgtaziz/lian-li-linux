import { defineStore } from "pinia";
import { ref } from "vue";
import { useIpc } from "@/composables/useIpc";

/**
 * AIO + wireless-bind side effects that are not part of the config save path.
 * Bind/unbind go directly to the daemon; AIO pump/fan/sensor/colour changes
 * are persisted via SetConfig in the config store.
 */
export const useAioStore = defineStore("aio", () => {
  const ipc = useIpc();
  const lastError = ref("");

  async function bindWireless(mac: string) {
    try {
      await ipc.request("BindWirelessDevice", { mac });
    } catch (e) {
      lastError.value = String(e);
    }
  }

  async function unbindWireless(mac: string) {
    try {
      await ipc.request("UnbindWirelessDevice", { mac });
    } catch (e) {
      lastError.value = String(e);
    }
  }

  return { lastError, bindWireless, unbindWireless };
});
