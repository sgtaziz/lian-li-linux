<script setup lang="ts">
import { computed } from "vue";
import { Monitor, Fan, Droplet, Palette, Loader2 } from "lucide-vue-next";
import type { DeviceInfo } from "@/types";
import { useDevicesStore } from "@/stores/devices";
import { useFansStore } from "@/stores/fans";
import { useLcdStore } from "@/stores/lcd";
import { useAioStore } from "@/stores/aio";
import { useConfigStore } from "@/stores/config";
import {
  FAMILY_DISPLAY,
  familyHasLcd,
  familyHasFan,
  familyHasPump,
  familyHasRgb,
  familySupportsDisplaySwitch,
  familyIsDesktopMode,
} from "@/constants";

const props = defineProps<{ device: DeviceInfo }>();

const devices = useDevicesStore();
const fans = useFansStore();
const lcd = useLcdStore();
const aio = useAioStore();
const config = useConfigStore();

const d = computed(() => props.device);
const familyName = computed(() => FAMILY_DISPLAY[d.value.family] ?? d.value.family);

const caps = computed(() => {
  const f = d.value.family;
  return {
    lcd: familyHasLcd(f) || d.value.has_lcd,
    fan: familyHasFan(f) || d.value.has_fan,
    pump: familyHasPump(f) || d.value.has_pump,
    rgb: familyHasRgb(f) || d.value.has_rgb,
  };
});

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

// ── Fan quantity stepper (ENE 6K77) ────────────────────────────────────────
const supportsFanQuantity = computed(
  () => (d.value.max_fan_quantity ?? 0) > 0 && d.value.has_fan,
);

const fanQty = computed({
  get: () => d.value.fan_quantity ?? 0,
  set: (v: number) => {
    const clamped = Math.max(0, Math.min(d.value.max_fan_quantity ?? 0, v));
    devices.pending.set(d.value.device_id, "fan-quantity");
    fans.scheduleFanQuantity(d.value.device_id, clamped);
  },
});

// Persist the fan quantity into the ENE6K77 config map for the next save.
function onFanQty(v: number | null) {
  if (v === null) return;
  const serial = d.value.serial ?? d.value.device_id;
  const ene = config.config.ene6k77[serial] ?? { fan_quantities: {} };
  ene.fan_quantities[d.value.device_id] = v;
  config.config.ene6k77[serial] = ene;
  config.markDirty();
  devices.pending.set(d.value.device_id, "fan-quantity");
  fans.scheduleFanQuantity(d.value.device_id, v);
}

// ── Display-mode switch ────────────────────────────────────────────────────
const supportsDisplaySwitch = computed(() =>
  familySupportsDisplaySwitch(d.value.family),
);
const isDesktop = computed(() => familyIsDesktopMode(d.value.family));
const displayModeLabel = computed(() =>
  isDesktop.value ? "Switch to LCD Mode" : "Switch to Desktop Mode",
);

async function onSwitchDisplay() {
  devices.pending.set(d.value.device_id, "switch");
  try {
    await lcd.switchDisplayMode(d.value.device_id);
  } finally {
    await refreshSoon();
  }
}

// ── Bind / unbind ──────────────────────────────────────────────────────────
const isUnboundWireless = computed(() => d.value.is_unbound_wireless);
const isBoundWireless = computed(() => d.value.device_id.startsWith("wireless:"));

async function onBind() {
  const mac = d.value.device_id.startsWith("wireless-unbound:")
    ? d.value.device_id.slice("wireless-unbound:".length)
    : d.value.device_id;
  devices.pending.set(d.value.device_id, "bind");
  await aio.bindWireless(mac);
  await refreshSoon();
}

async function onUnbind() {
  const mac = d.value.device_id.startsWith("wireless:")
    ? d.value.device_id.slice("wireless:".length)
    : d.value.device_id;
  devices.pending.set(d.value.device_id, "unbind");
  await aio.unbindWireless(mac);
  await refreshSoon();
}

async function refreshSoon() {
  const { useDaemonStore } = await import("@/stores/daemon");
  await useDaemonStore().refresh();
}

function badgeClass(kind: string) {
  return `badge-${kind}`;
}
</script>

<template>
  <div class="card device-card">
    <div class="head">
      <div class="name">{{ d.name }}</div>
      <div class="family">{{ familyName }}</div>
    </div>

    <div class="badges">
      <span v-if="caps.lcd" class="badge" :class="badgeClass('lcd')">
        <Monitor :size="11" /> LCD
      </span>
      <span v-if="caps.fan" class="badge" :class="badgeClass('fan')">
        <Fan :size="11" /> Fan
      </span>
      <span v-if="caps.pump" class="badge" :class="badgeClass('pump')">
        <Droplet :size="11" /> Pump
      </span>
      <span v-if="caps.rgb" class="badge" :class="badgeClass('rgb')">
        <Palette :size="11" /> RGB
      </span>
    </div>

    <div class="meta">
      <div v-if="d.serial" class="meta-row">
        <span class="muted">Serial</span>
        <span class="mono">{{ d.serial }}</span>
      </div>
      <div v-if="resolution" class="meta-row">
        <span class="muted">Screen</span>
        <span class="mono">{{ resolution }}</span>
      </div>
      <div v-if="fanRpmText" class="meta-row">
        <span class="muted">Fan RPM</span>
        <span class="mono">{{ fanRpmText }}</span>
      </div>
      <div v-if="coolantText" class="meta-row">
        <span class="muted">Coolant</span>
        <span class="mono">{{ coolantText }}</span>
      </div>
      <div v-if="d.firmware_version" class="meta-row">
        <span class="muted">Firmware</span>
        <span class="mono">{{ d.firmware_version }}</span>
      </div>
    </div>

    <!-- Fan quantity stepper (ENE 6K77) -->
    <div v-if="supportsFanQuantity" class="action-row">
      <span class="muted">Fan quantity</span>
      <n-input-number
        :value="fanQty"
        size="small"
        :min="0"
        :max="d.max_fan_quantity ?? 0"
        :disabled="pending === 'fan-quantity'"
        @update:value="onFanQty"
      />
    </div>

    <!-- Display-mode switch -->
    <div v-if="supportsDisplaySwitch" class="action-row">
      <n-button
        block
        size="small"
        :loading="pending === 'switch'"
        :disabled="pending === 'switch'"
        @click="onSwitchDisplay"
      >
        <template v-if="pending === 'switch'" #icon><Loader2 :size="14" class="spin" /></template>
        {{ pending === "switch" ? "Switching..." : displayModeLabel }}
      </n-button>
    </div>

    <!-- Bind / unbind -->
    <div class="action-row">
      <n-button
        v-if="isUnboundWireless"
        block
        size="small"
        type="primary"
        :loading="pending === 'bind'"
        :disabled="pending === 'bind'"
        @click="onBind"
      >
        <template v-if="pending === 'bind'" #icon><Loader2 :size="14" class="spin" /></template>
        {{ pending === "bind" ? "Binding..." : "Bind" }}
      </n-button>
      <n-button
        v-if="isBoundWireless"
        block
        size="small"
        :loading="pending === 'unbind'"
        :disabled="pending === 'unbind'"
        @click="onUnbind"
      >
        <template v-if="pending === 'unbind'" #icon><Loader2 :size="14" class="spin" /></template>
        {{ pending === "unbind" ? "Unbinding..." : "Unbind" }}
      </n-button>
    </div>
  </div>
</template>

<style scoped>
.device-card {
  /* Tighter than the default card padding to reduce empty space. */
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.head .name {
  font-weight: 600;
  font-size: var(--font-size-lg);
}
.head .family {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
.badges {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
}
.badge {
  font-size: var(--font-size-xs);
}
.badge-lcd {
  background: rgba(167, 139, 250, 0.15);
  color: var(--purple);
}
.badge-fan {
  background: rgba(79, 158, 255, 0.15);
  color: var(--accent);
}
.badge-pump {
  background: rgba(45, 212, 191, 0.15);
  color: var(--teal);
}
.badge-rgb {
  background: rgba(244, 114, 182, 0.15);
  color: var(--pink);
}
.meta {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.meta-row {
  display: flex;
  justify-content: space-between;
  font-size: var(--font-size-sm);
}
.mono {
  font-family: var(--font-mono);
}
.action-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.spin {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
