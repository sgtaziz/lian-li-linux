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
import { useDeviceNamesStore } from "@/stores/deviceNames";
import { useIpc } from "@/composables/useIpc";
import EditableName from "@/components/common/EditableName.vue";
import HistorySparkline from "./HistorySparkline.vue";
import { averageSpinningRpm } from "@/utils/telemetryHistory";
import { fanQuantityKey, fanQuantityPort, stageFanQuantity } from "@/utils/fanQuantity";
import {
  FAMILY_DISPLAY,
  familySupportsDisplaySwitch,
  familyIsDesktopMode,
} from "@/constants";
import { useTranslate } from "@/i18n";
import { backendText } from "@/utils/backendText";

const t = useTranslate();
const message = useMessage();
const props = defineProps<{ device: DeviceInfo }>();

const devices = useDevicesStore();
const fans = useFansStore();
const lcd = useLcdStore();
const aio = useAioStore();
const config = useConfigStore();
const daemon = useDaemonStore();
const names = useDeviceNamesStore();
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
const rpmHistory = computed(() => devices.history[d.value.device_id]?.rpm ?? []);
const currentRpm = computed(() => averageSpinningRpm(devices.fanSlotRpms(d.value, fanRpms.value)));
const coolantHistory = computed(() => devices.history[d.value.device_id]?.coolant ?? []);
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
  isDesktop.value ? t("devices.card.switchToLcd") : t("devices.card.switchToDesktop"),
);

async function onSwitchDisplay() {
  const deviceId = d.value.device_id;
  if (!devices.beginDisplaySwitch(d.value)) return;
  try {
    await lcd.switchDisplayMode(deviceId);
  } catch (error) {
    devices.finishDisplaySwitch(deviceId, { message: String(error) });
    message.error(String(error));
  } finally {
    await refreshSoon();
  }
}

// ── Bind / unbind ──────────────────────────────────────────────────────────
const isUnboundWireless = computed(() => d.value.is_unbound_wireless);
const isBoundWireless = computed(() => d.value.device_id.startsWith("wireless:"));
const isBindOther = computed(() => d.value.wireless_bind_status === "bind_other");
const needsRebind = computed(() => d.value.wireless_bind_status === "needs_rebind");
const foreignOnline = computed(() => d.value.foreign_master_online === true);
const bindLabel = computed(() => {
  if (pending.value === "bind") return t("devices.card.bindingButton");
  return isBindOther.value ? t("devices.card.takeOver") : t("devices.card.bind");
});

async function onBind() {
  const mac = d.value.device_id.replace(/^wireless(-unbound)?:/, "");
  if (isBindOther.value) {
    const ok = await new Promise<boolean>((resolve) => {
      dialog.warning({
        title: t("devices.card.takeOverTitle"),
        content: t("devices.card.takeOverContent"),
        positiveText: t("devices.card.takeOver"),
        negativeText: t("common.cancel"),
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

const defaultLabel = computed(() =>
  devices.labels.get(d.value.device_id) ?? d.value.name,
);
const customName = computed(() => names.nameOf(d.value.device_id));
const role = computed(() => {
  if (caps.value.fan && fanRpms.value.length > 1) return t("devices.card.roleFanGroup", { count: fanRpms.value.length });
  if (caps.value.lcd && !caps.value.fan && !caps.value.pump) return t("devices.card.roleLcdScreen");
  return null;
});

const subtitle = computed(() =>
  [
    role.value,
    familyName.value,
    isBoundWireless.value || isUnboundWireless.value ? t("devices.card.wireless") : null,
    d.value.serial ? t("devices.card.serial", { serial: d.value.serial }) : null,
    d.value.firmware_version ? t("devices.card.firmware", { version: d.value.firmware_version }) : null,
  ].filter(Boolean).join(" · "),
);

type StatusTone = "success" | "warning" | "danger" | "muted" | "info";
const status = computed<{ label: string; tone: StatusTone; busy?: boolean }>(() => {
  if (!daemon.connected) return { label: t("common.offline"), tone: "muted" };
  switch (pending.value) {
    case "switch": return { label: t("devices.card.status.switching"), tone: "warning", busy: true };
    case "bind": return { label: t("devices.card.status.binding"), tone: "warning", busy: true };
    case "unbind": return { label: t("devices.card.status.unbinding"), tone: "warning", busy: true };
    case "fan-quantity": return { label: t("devices.card.status.applying"), tone: "warning", busy: true };
  }
  if (isUnboundWireless.value) {
    return isBindOther.value
      ? { label: foreignOnline.value ? t("devices.card.status.otherPcOnline") : t("devices.card.status.otherPcOffline"), tone: "muted" }
      : { label: t("devices.card.status.notBound"), tone: "muted" };
  }
  if (needsRebind.value) return { label: t("devices.card.status.needsRebind"), tone: "warning" };
  if (d.value.telemetry?.error) return { label: t("devices.card.status.telemetryError"), tone: "danger" };
  if (isDesktop.value) return { label: t("devices.card.status.desktopMode"), tone: "info" };
  return { label: t("common.online"), tone: "success" };
});

const temperatureReadings = computed(() =>
  (d.value.telemetry?.temperatures ?? []).map((reading) => ({
    id: reading.name,
    name: backendText(reading.name, reading.name_ui),
    text: reading.celsius == null ? (reading.abnormal ? t("devices.card.abnormal") : t("devices.card.notAvailable")) : `${reading.celsius.toFixed(1)}°C`,
  })),
);

async function ping() {
  try {
    await ipc.request("PingDevice", { device_id: d.value.device_id, zone: 0 });
  } catch (e) {
    dialog.error({
      title: t("devices.card.pingFailed"),
      content: String(e),
      positiveText: t("common.ok"),
    });
  }
}
</script>

<template>
  <div class="device-row">
    <div class="icon-box"><component :is="primaryIcon" :size="18" /></div>

    <div class="identity">
      <div class="name-line">
        <EditableName
          :name="customName"
          :label="defaultLabel"
          @rename="(name) => names.setName(d.device_id, name)"
        />
        <span class="caps">
          <Monitor v-if="caps.lcd" :size="12" class="cap-lcd" title="LCD" />
          <Fan v-if="caps.fan" :size="12" class="cap-fan" :title="t('devices.card.capFan')" />
          <Droplet v-if="caps.pump" :size="12" class="cap-pump" :title="t('devices.card.capPump')" />
          <Palette v-if="caps.rgb" :size="12" class="cap-rgb" title="RGB" />
        </span>
      </div>
      <div class="subtitle">{{ subtitle }}</div>
      <p v-if="d.telemetry?.error" class="error-text">{{ backendText(d.telemetry.error, d.telemetry.error_ui) }}</p>
      <p v-if="isUnboundWireless && isBindOther" class="subtitle">{{ t("devices.card.ownedByOther") }}</p>
      <p v-if="isBoundWireless && needsRebind" class="subtitle">{{ t("devices.card.needsRebindHint") }}</p>
    </div>

    <span class="status" :class="`tone-${status.tone}`">
      <Loader2 v-if="status.busy" :size="11" class="spin" />
      <span v-else class="dot" />
      {{ status.label }}
    </span>

    <div class="readings">
      <span v-if="fanRpmText" class="reading" :title="t('devices.card.fanRpmTitle', { rpms: fanRpmText })">
        <Fan :size="12" /><span class="mono">{{ fanRpmText }}</span>
      </span>
      <span v-if="coolantText" class="reading" :title="t('devices.card.coolantTitle')">
        <Droplet :size="12" /><span class="mono">{{ coolantText }}</span>
      </span>
      <span v-for="reading in temperatureReadings" :key="reading.id" class="reading" :title="reading.name">
        <span class="reading-label">{{ reading.name }}</span><span class="mono">{{ reading.text }}</span>
      </span>
      <span v-if="resolution" class="reading" :title="t('devices.card.screenTitle')">
        <Monitor :size="12" /><span class="mono">{{ resolution }}</span>
      </span>
    </div>

    <div v-if="rpmHistory.length || coolantHistory.length" class="histories">
      <span class="histories-title">{{ t("devices.history.window") }}</span>
      <HistorySparkline v-if="rpmHistory.length" :samples="rpmHistory" :current="currentRpm" :label="t('devices.history.fanSpeed')" unit=" rpm" :digits="0" tone="rpm" />
      <HistorySparkline v-if="coolantHistory.length" :samples="coolantHistory" :current="coolant" :label="t('devices.history.coolant')" unit=" °C" :digits="1" tone="coolant" />
    </div>

    <div class="actions">
      <label v-if="supportsFanQuantity" class="qty">
        <span class="muted">{{ t("devices.card.fanQuantity") }}</span>
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
      >{{ pending === "switch" ? t("devices.card.status.switching") : displayModeLabel }}</n-button>
      <n-button
        v-if="isUnboundWireless"
        size="tiny"
        :type="isBindOther ? 'warning' : 'primary'"
        :loading="pending === 'bind'"
        :disabled="pending === 'bind' || (isBindOther && foreignOnline)"
        @click="onBind"
      >{{ bindLabel }}</n-button>
      <n-button
        v-if="isBoundWireless && needsRebind"
        size="tiny"
        type="warning"
        :loading="pending === 'bind'"
        :disabled="pending === 'bind' || pending === 'unbind'"
        @click="onBind"
      >{{ pending === "bind" ? t("devices.card.rebinding") : t("devices.card.rebind") }}</n-button>
      <n-button
        v-if="isBoundWireless"
        size="tiny"
        :loading="pending === 'unbind'"
        :disabled="pending === 'unbind' || pending === 'bind'"
        @click="onUnbind"
      >{{ pending === "unbind" ? t("devices.card.status.unbinding") : t("devices.card.unbind") }}</n-button>
      <button v-if="caps.rgb" class="ping-btn" :title="t('devices.card.identify')" @click="ping">
        <Locate :size="15" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.device-row {
  display: grid;
  grid-template-columns: 56px minmax(0, 1fr) auto;
  grid-template-areas:
    "icon identity status"
    "icon readings actions"
    "history history history";
  align-items: start;
  gap: var(--space-2) var(--space-4);
  padding: 20px;
  border-radius: 28px;
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface-container-low);
  transition: border-color 0.2s, background 0.2s;
}
.device-row:hover {
  border-color: var(--md-outline);
}
.icon-box {
  grid-area: icon;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: 18px;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.identity {
  grid-area: identity;
  align-self: center;
}
.status {
  grid-area: status;
  align-self: center;
}
.readings {
  grid-area: readings;
}
.actions {
  grid-area: actions;
}
.histories {
  grid-area: history;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 180px), 1fr));
  gap: var(--space-2);
  margin-top: var(--space-2);
}
.histories-title {
  grid-column: 1 / -1;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.4px;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}
.actions:empty {
  display: none;
}
.identity {
  min-width: 0;
}
.name-line {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.caps {
  display: inline-flex;
  gap: 4px;
  flex-shrink: 0;
}
.caps svg {
  box-sizing: content-box;
  padding: 3px;
  border-radius: 6px;
}
.cap-lcd { color: var(--md-on-secondary-container); background: var(--md-secondary-container); }
.cap-fan { color: var(--md-on-primary-container); background: var(--md-primary-container); }
.cap-pump { color: var(--md-on-tertiary-container); background: var(--md-tertiary-container); }
.cap-rgb { color: var(--md-on-surface); background: var(--md-surface-container-highest); }
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
  gap: var(--space-1) var(--space-4);
  min-width: 0;
  padding-top: var(--space-3);
  border-top: 1px solid var(--md-outline-variant);
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.readings:empty {
  display: none;
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
  padding-top: var(--space-2);
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
</style>
