import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { usePolling } from "@/composables/usePolling";
import { useDevicesStore } from "@/stores/devices";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import { useLcdStore } from "@/stores/lcd";
import { DONGLE_FAMILIES } from "@/constants";
import type { DaemonInfo, SensorInfo, TelemetrySnapshot } from "@/types";

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

export const useDaemonStore = defineStore("daemon", () => {
  const ipc = useIpc();
  const devices = useDevicesStore();
  const config = useConfigStore();
  const thermal = useThermalStore();
  const lcd = useLcdStore();

  const connected = ref(false);
  const socketPath = ref("");
  const streamingActive = ref(false);
  const mediaPreparation = ref<NonNullable<TelemetrySnapshot["media_preparation"]>>({});
  const desktopStreams = ref<NonNullable<TelemetrySnapshot["desktop_streams"]>>([]);
  const info = ref<DaemonInfo | null>(null);
  const writeError = ref<string | null>(null);
  const canWrite = computed(() => connected.value && !writeError.value);
  const version = computed(() => info.value?.version ?? "");
  const openrgbRunning = ref(false);
  const openrgbEnabled = ref(false);
  const openrgbError = ref("");
  const openrgbPort = ref<number | null>(null);

  let wasConnected = false;
  let lastDeviceCount = -1;

  async function tick() {
    try {
      const result = await ipc.poll();
      const previousInstance = info.value?.instance_id;
      const previousSocket = socketPath.value;
      info.value = result.daemon_info ?? null;
      connected.value = result.connected;
      writeError.value = result.write_error ?? null;
      socketPath.value = result.socket_path;
      streamingActive.value = result.telemetry.streaming_active;
      mediaPreparation.value = result.telemetry.media_preparation ?? {};
      desktopStreams.value = result.telemetry.desktop_streams ?? [];
      openrgbRunning.value = result.telemetry.openrgb_status.running;
      openrgbEnabled.value = result.telemetry.openrgb_status.enabled;
      openrgbError.value = result.telemetry.openrgb_status.error ?? "";
      openrgbPort.value = result.telemetry.openrgb_status.port;

      devices.applyPoll(result.devices, result.telemetry);
      lcd.applyCleanerTelemetry(result.telemetry.pixel_clean_statuses);

      if (result.connected) {
        const visible = result.devices.filter(
          (d) => !DONGLE_FAMILIES.includes(d.family),
        ).length;
        if (!wasConnected || previousInstance !== info.value?.instance_id ||
            previousSocket !== result.socket_path || visible !== lastDeviceCount) {
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
      mediaPreparation.value = {};
      desktopStreams.value = [];
      info.value = null;
      writeError.value = null;
      wasConnected = false;
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
    writeError,
    canWrite,
    socketPath,
    streamingActive,
    mediaPreparation,
    desktopStreams,
    version,
    info,
    openrgbRunning,
    openrgbEnabled,
    openrgbError,
    openrgbPort,
    refresh,
    start,
    stop,
  };
});
