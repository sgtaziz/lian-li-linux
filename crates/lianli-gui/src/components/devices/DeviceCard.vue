<script setup lang="ts">
import { computed } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { Monitor, Fan, Droplet, Palette, Loader2, Locate } from "lucide-vue-next";
import type { DeviceInfo } from "@/types";
import { useDevicesStore } from "@/stores/devices";
import { useFansStore } from "@/stores/fans";
import { useLcdStore } from "@/stores/lcd";
import { useAioStore } from "@/stores/aio";
import { useConfigStore } from "@/stores/config";
import { useDaemonStore } from "@/stores/daemon";
import { useLcdNamesStore } from "@/stores/lcdNames";
import { lcdDeviceLabels, lcdEntryKey, resolveLcdDevice } from "@/utils/lcdSelection";
import { useIpc } from "@/composables/useIpc";
import { fanQuantityKey, fanQuantityPort, stageFanQuantity } from "@/utils/fanQuantity";
import {
  FAMILY_DISPLAY,
  familySupportsDisplaySwitch,
  familyIsDesktopMode,
} from "@/constants";

const message = useMessage();
const props = defineProps<{ device: DeviceInfo }>();

const devices = useDevicesStore();
const fans = useFansStore();
const lcd = useLcdStore();
const aio = useAioStore();
const config = useConfigStore();
const daemon = useDaemonStore();
const lcdNames = useLcdNamesStore();
const dialog = useDialog();
const ipc = useIpc();

const d = computed(() => props.device);
const familyName = computed(() => FAMILY_DISPLAY[d.value.family] ?? d.value.family);

const caps = computed(() => ({
  lcd: d.value.has_lcd,
  fan: d.value.has_fan,
  pump: d.value.has_pump,
  rgb: d.value.has_rgb,
}));

const resolution = computed(() => {
  const { screen_width: w, screen_height: h } = d.value;
  return w && h ? `${w}x${h}` : "";
});

const fanRpms = computed(() => devices.fanRpms(d.value.device_id));
const coolant = computed(() => devices.coolantTemp(d.value.device_id));
const fanRpmText = computed(() =>
  fanRpms.value.length ? fanRpms.value.join(", ") : "",
);
const coolantText = computed(() =>
  coolant.value !== null ? `${coolant.value.toFixed(1)}\u00B0C` : "",
);

const pending = computed(() => devices.pending.get(d.value.device_id));

const supportsFanQuantity = computed(
  () => (d.value.max_fan_quantity ?? 0) > 0 && d.value.has_fan,
);

const quantityPort = computed(() => fanQuantityPort(d.value.device_id));
const fanQty = computed(() => config.config.ene6k77[fanQuantityKey(d.value.device_id)]?.fan_quantities[quantityPort.value ?? ""] ?? d.value.fan_quantity ?? 0);

function onFanQty(v: number | null) {
  if (v === null) return;
  const quantity = stageFanQuantity(config.config, d.value, v);
  if (quantity === undefined) return;
  config.markDirty();
  devices.pending.set(d.value.device_id, "fan-quantity");
  const deviceId = d.value.device_id;
  fans.scheduleFanQuantity(deviceId, quantity, (error) => {
    devices.pending.clear(deviceId);
    if (error) message.error(String(error));
  });
}

const supportsDisplaySwitch = computed(() =>
  familySupportsDisplaySwitch(d.value.family),
);
const isDesktop = computed(() => familyIsDesktopMode(d.value.family));
const displayModeLabel = computed(() =>
  isDesktop.value ? "Switch to LCD Mode" : "Switch to Desktop Mode",
);

async function onSwitchDisplay() {
  const deviceId = d.value.device_id;
  if (!devices.beginDisplaySwitch(d.value)) return;
  try {
    await lcd.switchDisplayMode(deviceId);
  } catch (error) {
    devices.finishDisplaySwitch(deviceId, String(error));
    message.error(String(error));
  } finally {
    await refreshSoon();
  }
}

// ── Bind / unbind ──────────────────────────────────────────────────────────
const isUnboundWireless = computed(() => d.value.is_unbound_wireless);
const isBoundWireless = computed(() => d.value.device_id.startsWith("wireless:"));
const isBindOther = computed(() => d.value.wireless_bind_status === "bind_other");
const foreignOnline = computed(() => d.value.foreign_master_online === true);
const bindLabel = computed(() => {
  if (pending.value === "bind") return "Binding...";
  return isBindOther.value ? "Take over" : "Bind";
});

async function onBind() {
  const mac = d.value.device_id.startsWith("wireless-unbound:")
    ? d.value.device_id.slice("wireless-unbound:".length)
    : d.value.device_id;
  if (isBindOther.value) {
    const ok = await new Promise<boolean>((resolve) => {
      dialog.warning({
        title: "Take over this device?",
        content:
          "It currently belongs to another controller. Taking it over will disconnect it from that PC.",
        positiveText: "Take over",
        negativeText: "Cancel",
        onPositiveClick: () => resolve(true),
        onNegativeClick: () => resolve(false),
        onClose: () => resolve(false),
        onMaskClick: () => resolve(false),
      });
    });
    if (!ok) return;
  }
  const pendingId = d.value.device_id;
  devices.pending.set(pendingId, "bind", true);
  try {
    await aio.bindWireless(mac);
  } catch (error) {
    message.error(String(error));
  } finally {
    devices.pending.clear(pendingId);
    await refreshSoon();
  }
}

async function onUnbind() {
  const mac = d.value.device_id.startsWith("wireless:")
    ? d.value.device_id.slice("wireless:".length)
    : d.value.device_id;
  const pendingId = d.value.device_id;
  devices.pending.set(pendingId, "unbind", true);
  try {
    await aio.unbindWireless(mac);
  } catch (error) {
    message.error(String(error));
  } finally {
    devices.pending.clear(pendingId);
    await refreshSoon();
  }
}

async function refreshSoon() {
  await daemon.refresh();
}

const primaryIcon = computed(() => {
  if (caps.value.lcd) return Monitor;
  if (caps.value.pump) return Droplet;
  if (caps.value.fan) return Fan;
  return Palette;
});

const title = computed(() =>
  caps.value.lcd ? lcdDeviceLabels(devices.lcdDevices).get(d.value.device_id) ?? d.value.name : d.value.name,
);
const lcdName = computed(() => {
  const entry = config.config.lcds.find(
    (candidate) => resolveLcdDevice(candidate, devices.lcdDevices)?.device_id === d.value.device_id,
  );
  return entry ? lcdNames.names[lcdEntryKey(entry)] ?? "" : "";
});
const role = computed(() => {
  if (caps.value.fan && fanRpms.value.length > 1) return `Fan group · ${fanRpms.value.length} fans`;
  if (caps.value.lcd && !caps.value.fan && !caps.value.pump) {
    return lcdName.value ? `LCD screen · ${lcdName.value}` : "LCD screen";
  }
  return null;
});

const subtitle = computed(() =>
  [
    role.value,
    familyName.value,
    isBoundWireless.value || isUnboundWireless.value ? "Wireless" : null,
    d.value.serial ? `SN ${d.value.serial}` : null,
    d.value.firmware_version ? `FW ${d.value.firmware_version}` : null,
  ].filter(Boolean).join(" · "),
);

type StatusTone = "success" | "warning" | "danger" | "muted" | "info";
const status = computed<{ label: string; tone: StatusTone; busy?: boolean }>(() => {
  if (!daemon.connected) return { label: "Offline", tone: "muted" };
  switch (pending.value) {
    case "switch": return { label: "Switching…", tone: "warning", busy: true };
    case "bind": return { label: "Binding…", tone: "warning", busy: true };
    case "unbind": return { label: "Unbinding…", tone: "warning", busy: true };
    case "fan-quantity": return { label: "Applying…", tone: "warning", busy: true };
  }
  if (isUnboundWireless.value) {
    return isBindOther.value
      ? { label: foreignOnline.value ? "Other PC (online)" : "Other PC (offline)", tone: "muted" }
      : { label: "Not bound", tone: "muted" };
  }
  if (d.value.telemetry?.error) return { label: "Telemetry error", tone: "danger" };
  if (isDesktop.value) return { label: "Desktop mode", tone: "info" };
  return { label: "Online", tone: "success" };
});

const temperatureReadings = computed(() =>
  (d.value.telemetry?.temperatures ?? []).map((reading) => ({
    name: reading.name,
    text: reading.celsius == null ? (reading.abnormal ? "Abnormal" : "N/A") : `${reading.celsius.toFixed(1)}°C`,
  })),
);

async function ping() {
  try {
    await ipc.request("PingDevice", { device_id: d.value.device_id, zone: 0 });
  } catch (e) {
    dialog.error({
      title: "Ping failed",
      content: String(e),
      positiveText: "OK",
    });
  }
}
</script>

<template>
  <div class="device-row">
    <div class="icon-box"><component :is="primaryIcon" :size="18" /></div>

    <div class="identity">
      <div class="name-line">
        <span class="name">{{ title }}</span>
        <span class="caps">
          <Monitor v-if="caps.lcd" :size="12" class="cap-lcd" title="LCD" />
          <Fan v-if="caps.fan" :size="12" class="cap-fan" title="Fan" />
          <Droplet v-if="caps.pump" :size="12" class="cap-pump" title="Pump" />
          <Palette v-if="caps.rgb" :size="12" class="cap-rgb" title="RGB" />
        </span>
      </div>
      <div class="subtitle">{{ subtitle }}</div>
      <p v-if="d.telemetry?.error" class="error-text">{{ d.telemetry.error }}</p>
      <p v-if="isUnboundWireless && isBindOther" class="subtitle">Owned by another controller</p>
    </div>

    <span class="status" :class="`tone-${status.tone}`">
      <Loader2 v-if="status.busy" :size="11" class="spin" />
      <span v-else class="dot" />
      {{ status.label }}
    </span>

    <div class="readings">
      <span v-if="fanRpmText" class="reading" :title="`Fan RPM: ${fanRpmText}`">
        <Fan :size="12" /><span class="mono">{{ fanRpmText }}</span>
      </span>
      <span v-if="coolantText" class="reading" title="Coolant">
        <Droplet :size="12" /><span class="mono">{{ coolantText }}</span>
      </span>
      <span v-for="reading in temperatureReadings" :key="reading.name" class="reading" :title="reading.name">
        <span class="reading-label">{{ reading.name }}</span><span class="mono">{{ reading.text }}</span>
      </span>
      <span v-if="resolution" class="reading" title="Screen">
        <Monitor :size="12" /><span class="mono">{{ resolution }}</span>
      </span>
    </div>

    <div class="actions">
      <label v-if="supportsFanQuantity" class="qty">
        <span class="muted">Fans</span>
        <n-input-number
          :value="fanQty"
          size="tiny"
          :min="0"
          :max="d.max_fan_quantity ?? 0"
          :disabled="pending === 'fan-quantity'"
          @update:value="onFanQty"
        />
      </label>
      <n-button
        v-if="supportsDisplaySwitch"
        size="tiny"
        :loading="pending === 'switch'"
        :disabled="pending === 'switch'"
        @click="onSwitchDisplay"
      >{{ pending === "switch" ? "Switching…" : displayModeLabel }}</n-button>
      <n-button
        v-if="isUnboundWireless"
        size="tiny"
        :type="isBindOther ? 'warning' : 'primary'"
        :loading="pending === 'bind'"
        :disabled="pending === 'bind' || (isBindOther && foreignOnline)"
        @click="onBind"
      >{{ bindLabel }}</n-button>
      <n-button
        v-if="isBoundWireless"
        size="tiny"
        :loading="pending === 'unbind'"
        :disabled="pending === 'unbind'"
        @click="onUnbind"
      >{{ pending === "unbind" ? "Unbinding…" : "Unbind" }}</n-button>
      <button v-if="caps.rgb" class="ping-btn" title="Identify device" @click="ping">
        <Locate :size="15" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.device-row {
  display: grid;
  grid-template-columns: 36px minmax(180px, 1.4fr) 130px minmax(0, 1.6fr) auto;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
}
.device-row + .device-row {
  border-top: 1px solid var(--border);
}
.icon-box {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
  color: var(--text-secondary);
}
.identity {
  min-width: 0;
}
.name-line {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.caps {
  display: inline-flex;
  gap: 4px;
  flex-shrink: 0;
}
.cap-lcd { color: var(--purple); }
.cap-fan { color: var(--accent); }
.cap-pump { color: var(--teal); }
.cap-rgb { color: var(--pink); }
.subtitle {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.error-text {
  margin: 2px 0 0;
  font-size: var(--font-size-xs);
  color: var(--danger);
  overflow-wrap: anywhere;
}
.status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  width: fit-content;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: var(--font-size-xs);
  white-space: nowrap;
  background: color-mix(in srgb, currentColor 12%, transparent);
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.tone-success { color: var(--success); }
.tone-warning { color: var(--warning); }
.tone-danger { color: var(--danger); }
.tone-info { color: var(--accent); }
.tone-muted { color: var(--text-muted); }
.readings {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1) var(--space-3);
  min-width: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
.reading {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  max-width: 100%;
}
.reading .mono {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
}
.reading-label {
  font-size: var(--font-size-xs);
}
.mono {
  font-family: var(--font-mono);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-2);
}
.qty {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--font-size-xs);
}
.qty .n-input-number {
  width: 84px;
}
.ping-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.ping-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}
.spin {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to { transform: rotate(360deg); }
}
@container (max-width: 760px) {
  .device-row {
    grid-template-columns: 36px minmax(0, 1fr) auto;
  }
  .readings {
    grid-column: 2 / -1;
  }
  .actions {
    grid-column: 2 / -1;
    justify-content: flex-start;
  }
}
</style>
