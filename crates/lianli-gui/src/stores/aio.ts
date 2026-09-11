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

  async function changeBinding(mac: string, bind: boolean) {
    lastError.value = "";
    try {
      const queued = await ipc.request<{ operation_id?: string }>(
        bind ? "BindWirelessDevice" : "UnbindWirelessDevice",
        { mac },
      );
      if (queued.operation_id == null) return;
      const deadline = Date.now() + 60_000;
      while (Date.now() < deadline) {
        const result = await ipc.request<{ status: string; message?: string }>(
          "GetWirelessOperation",
          { operation_id: queued.operation_id },
        );
        if (result.status === "succeeded") return;
        if (result.status === "failed") {
          throw new Error(result.message || "Wireless operation failed");
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      throw new Error(
        "Wireless operation is still pending; refresh the device list to check its state.",
      );
    } catch (e) {
      lastError.value = String(e);
      throw e;
    }
  }

  async function bindWireless(mac: string) {
    await changeBinding(mac, true);
  }

  async function unbindWireless(mac: string) {
    await changeBinding(mac, false);
  }

  return { lastError, bindWireless, unbindWireless };
});
