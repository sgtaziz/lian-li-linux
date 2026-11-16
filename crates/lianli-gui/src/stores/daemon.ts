import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { useIpc } from "@/composables/useIpc";
import { usePolling } from "@/composables/usePolling";
import { useDevicesStore } from "@/stores/devices";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import { useLcdStore } from "@/stores/lcd";
import { DONGLE_FAMILIES } from "@/constants";
import type { DaemonInfo, SensorInfo, TelemetrySnapshot, WirelessDongle } from "@/types";
import type { UiMessage } from "@/utils/backendText";

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

const FOREGROUND_POLL_MS = 2000;
const BACKGROUND_POLL_MS = 10000;

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
  const writeErrorUi = ref<UiMessage | null>(null);
  const canWrite = computed(() => connected.value && !writeError.value);
  const version = computed(() => info.value?.version ?? "");
  const openrgbRunning = ref(false);
  const openrgbEnabled = ref(false);
  const openrgbError = ref("");
  const openrgbErrorUi = ref<UiMessage | null>(null);
  const openrgbPort = ref<number | null>(null);
  const unresponsiveDongle = ref<WirelessDongle | null>(null);

  const ready = ref(false);
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
      writeErrorUi.value = result.write_error_ui ?? null;
      socketPath.value = result.socket_path;
      streamingActive.value = result.telemetry.streaming_active;
      mediaPreparation.value = result.telemetry.media_preparation ?? {};
      desktopStreams.value = result.telemetry.desktop_streams ?? [];
      openrgbRunning.value = result.telemetry.openrgb_status.running;
      openrgbEnabled.value = result.telemetry.openrgb_status.enabled;
      openrgbError.value = result.telemetry.openrgb_status.error ?? "";
      openrgbErrorUi.value = result.telemetry.openrgb_status.error_ui ?? null;
      openrgbPort.value = result.telemetry.openrgb_status.port;
      unresponsiveDongle.value = result.telemetry.unresponsive_dongle ?? null;

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
      unresponsiveDongle.value = null;
      info.value = null;
      writeError.value = null;
      writeErrorUi.value = null;
      wasConnected = false;
      // eslint-disable-next-line no-console
      console.warn("poll failed", e);
    } finally {
      ready.value = true;
    }
  }

  const polling = usePolling(tick, FOREGROUND_POLL_MS);

  async function refresh() {
    await polling.tick();
  }

  function start() {
    polling.start();
  }

  function stop() {
    polling.stop();
  }

  function setBackground(background: boolean) {
    polling.setIntervalMs(background ? BACKGROUND_POLL_MS : FOREGROUND_POLL_MS);
  }

  return {
    ready,
    connected,
    writeError,
    writeErrorUi,
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
    openrgbErrorUi,
    openrgbPort,
    unresponsiveDongle,
    refresh,
    start,
    stop,
    setBackground,
  };
});
