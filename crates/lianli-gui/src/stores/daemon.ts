import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { usePolling } from "@/composables/usePolling";
import { useDevicesStore } from "@/stores/devices";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import { DONGLE_FAMILIES } from "@/constants";
import type { SensorInfo } from "@/types";

function findTemp(sensors: SensorInfo[], kind: "cpu" | "gpu"): number | null {
  const match = sensors.find((s) => {
    if (s.unit !== "C") return false;
    const name = (s.display_name ?? "").toLowerCase();
    if (kind === "cpu") return name.includes("cpu") || name.includes("core");
    return (
      name.includes("gpu") ||
      (s.source.type === "nvidia_gpu" && (s.source as any).metric === "temp")
    );
  });
  return match?.current_value ?? null;
}

/**
 * Owns the 2s polling loop and connection state. Each tick runs
 * Ping + ListDevices + GetTelemetry, then fans the results out to the
 * devices store and (on reconnect / device-count change) triggers a config
 * reload — matching the Slint backend thread behaviour.
 */
export const useDaemonStore = defineStore("daemon", () => {
  const ipc = useIpc();
  const devices = useDevicesStore();
  const config = useConfigStore();
  const thermal = useThermalStore();

  const connected = ref(false);
  const socketPath = ref("");
  const streamingActive = ref(false);
  const version = ref("");
  const openrgbRunning = ref(false);
  const openrgbError = ref("");
  const openrgbPort = ref<number | null>(null);

  let wasConnected = false;
  let lastDeviceCount = -1;

  const visibleDeviceCount = computed(() => devices.list.length);

  async function tick() {
    try {
      const result = await ipc.poll();
      connected.value = result.connected;
      socketPath.value = result.socket_path;
      streamingActive.value = result.telemetry.streaming_active;
      openrgbRunning.value = result.telemetry.openrgb_status.running;
      openrgbError.value = result.telemetry.openrgb_status.error ?? "";
      openrgbPort.value = result.telemetry.openrgb_status.port;

      devices.applyPoll(result.devices, result.telemetry);

      if (result.connected) {
        const visible = result.devices.filter(
          (d) => !DONGLE_FAMILIES.includes(d.family),
        ).length;
        if (!wasConnected) {
          // Daemon reconnected — full config reload.
          await config.load();
        } else if (visible !== lastDeviceCount) {
          // Device set changed while connected — daemon may still be opening
          // devices, so reload config + capabilities.
          await config.load();
        }
        lastDeviceCount = visible;

        if (config.config.thermal_alert.cpu.enabled || config.config.thermal_alert.gpu.enabled) {
          try {
            const sensors = await ipc.request<SensorInfo[]>("ListSensors");
            thermal.setTemps(findTemp(sensors, "cpu"), findTemp(sensors, "gpu"));
          } catch {
            // sensor read failure — thermal status just stays stale
          }
        }
      }
      wasConnected = result.connected;
    } catch (e) {
      connected.value = false;
      // eslint-disable-next-line no-console
      console.warn("poll failed", e);
    }
  }

  const polling = usePolling(tick, 2000);

  async function refresh() {
    await polling.tick();
  }

  function start() {
    polling.start();
  }

  function stop() {
    polling.stop();
  }

  return {
    connected,
    socketPath,
    streamingActive,
    version,
    openrgbRunning,
    openrgbError,
    openrgbPort,
    visibleDeviceCount,
    refresh,
    start,
    stop,
  };
});
