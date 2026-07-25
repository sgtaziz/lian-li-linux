import { defineStore } from "pinia";
import { computed, ref } from "vue";
import type { DeviceInfo, TelemetrySnapshot } from "@/types";
import { DONGLE_FAMILIES } from "@/constants";
import { usePendingAction } from "@/composables/usePendingAction";

/// Dev-only mock AIO device for the AIO page.
const MOCK_AIO = false;

/// Fake wireless AIO so the AIO page can be tested without hardware.
const MOCK_AIO_DEVICE: DeviceInfo = {
  device_id: "wireless:de:ad:be:ef:00:01",
  family: "WirelessAio",
  name: "Mock HydroShift AIO",
  serial: "MOCK-AIO",
  vid: 0,
  pid: 0,
  has_lcd: false,
  has_fan: true,
  has_pump: true,
  has_rgb: true,
  has_pump_control: true,
  fan_count: 3,
  per_fan_control: false,
  mb_sync_support: false,
  rgb_zone_count: null,
  screen_width: null,
  screen_height: null,
  is_unbound_wireless: false,
  pump_rpm_range: [2200, 4200],
  fan_quantity: null,
  max_fan_quantity: null,
  firmware_version: "0.0.0-mock",
  supports_c_command: false,
  port_index: null,
};

/**
 * Holds the live device list + telemetry snapshot, refreshed every 2s by the
 * daemon store. Also owns pending-action tracking for the Devices page cards.
 */
export const useDevicesStore = defineStore("devices", () => {
  const list = ref<DeviceInfo[]>([]);
  const telemetry = ref<TelemetrySnapshot>({
    fan_rpms: {},
    coolant_temps: {},
    streaming_active: false,
    openrgb_status: { enabled: false, running: false, port: null, error: null },
  });
  const pending = usePendingAction();

  const visible = computed(() =>
    list.value.filter((d) => !DONGLE_FAMILIES.includes(d.family)),
  );

  /** Device lookup by id. */
  function byId(id: string): DeviceInfo | undefined {
    return list.value.find((d) => d.device_id === id);
  }

  /** Devices that expose an LCD screen. */
  const lcdDevices = computed(() => visible.value.filter((d) => d.has_lcd));

  /** Devices that have controllable fans (excluding AIOs handled on the AIO page). */
  const fanDevices = computed(() =>
    visible.value.filter((d) => d.has_fan && (d.fan_count ?? 0) > 0),
  );

  /** Devices whose family is an AIO (routed to the AIO page). */
  const aioDevices = computed(() => {
    const real = visible.value.filter((d) => {
      const fam = d.family;
      return (
        fam === "Galahad2Trinity" ||
        fam === "HydroShiftLcd" ||
        fam === "Galahad2Lcd" ||
        fam === "HydroShift2Lcd" ||
        fam === "HydroShift2OledCurveLed" ||
        fam === "WirelessAio"
      );
    });
    // DEV ONLY: inject a mock AIO device so the AIO page can be exercised
    // without hardware. Flip MOCK_AIO off (or build in release) to remove.
    return import.meta.env.DEV && MOCK_AIO ? [...real, MOCK_AIO_DEVICE] : real;
  });

  function fanRpms(deviceId: string): number[] {
    return telemetry.value.fan_rpms[deviceId] ?? [];
  }

  function coolantTemp(deviceId: string): number | null {
    return telemetry.value.coolant_temps[deviceId] ?? null;
  }

  function applyPoll(devices: DeviceInfo[], snap: TelemetrySnapshot) {
    list.value = devices;
    telemetry.value = snap;
    pending.expire(devices.map((d) => d.device_id));
  }

  return {
    list,
    telemetry,
    visible,
    lcdDevices,
    fanDevices,
    aioDevices,
    pending,
    byId,
    fanRpms,
    coolantTemp,
    applyPoll,
  };
});
