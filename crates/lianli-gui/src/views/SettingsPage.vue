<script setup lang="ts">
import { computed, ref, onMounted, watch } from "vue";
import OpenRgbRegionsToggle from "@/components/rgb/OpenRgbRegionsToggle.vue";
import { invoke } from "@tauri-apps/api/core";
import { ExternalLink } from "lucide-vue-next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import StatusDot from "@/components/common/StatusDot.vue";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import ServiceStatus from "@/components/common/ServiceStatus.vue";
import StateBackups from "@/components/common/StateBackups.vue";
import CatalogStorage from "@/components/common/CatalogStorage.vue";
import { useIpc } from "@/composables/useIpc";

const REPO_URL = "https://github.com/sgtaziz/lian-li-linux";
const daemon = useDaemonStore();
const config = useConfigStore();
const thermal = useThermalStore();
const ipc = useIpc();
const retryingOpenRgb = ref(false);
const openrgbFeedback = ref("");
watch(() => daemon.info?.instance_id, () => { openrgbFeedback.value = ""; });
async function retryOpenRgb() {
  if (retryingOpenRgb.value || !daemon.canWrite) return;
  const instance = daemon.info?.instance_id;
  retryingOpenRgb.value = true;
  openrgbFeedback.value = "";
  try {
    await ipc.request("RetryOpenRgb");
    if (daemon.info?.instance_id !== instance) return;
    openrgbFeedback.value = "Retry queued using the saved OpenRGB settings. Check the server status for the result.";
    await daemon.refresh();
  } catch (error) { if (daemon.info?.instance_id === instance) openrgbFeedback.value = String(error); }
  finally { retryingOpenRgb.value = false; }
}

const appVersion = ref("...");
onMounted(async () => {
  appVersion.value = await invoke<string>("app_version");
});

const rgb = computed(() => config.ensureRgb());
const fanCfg = computed(() => config.ensureFans());

const openrgbEnabled = computed({
  get: () => rgb.value.openrgb_server,
  set: (v: boolean) => {
    rgb.value.openrgb_server = v;
    config.markDirty();
  },
});
const openrgbPort = computed({
  get: () => rgb.value.openrgb_port,
  set: (v: number) => {
    rgb.value.openrgb_port = v;
    config.markDirty();
  },
});
const openrgbStatus = computed(() => {
  if (!daemon.connected) return "Disconnected";
  if (!daemon.openrgbEnabled) return "Disabled";
  if (daemon.openrgbError) return "Error";
  if (daemon.openrgbRunning) return daemon.openrgbPort === null ? "Running" : `Port ${daemon.openrgbPort}`;
  return "Starting…";
});
const openrgbDot = computed<"danger" | "success" | "warning" | "muted">(() =>
  !daemon.connected || !daemon.openrgbEnabled
    ? "muted"
    : daemon.openrgbError
      ? "danger"
      : daemon.openrgbRunning
        ? "success"
        : "warning",
);

const cpu = computed(() => config.config.thermal_alert.cpu);
const gpu = computed(() => config.config.thermal_alert.gpu);

const thermalStatusText = computed(() => {
  switch (thermal.status) {
    case "active":
      return "Active";
    case "monitoring":
      return "Monitoring";
    default:
      return "Disabled";
  }
});
const thermalStatusColor = computed<"danger" | "success" | "muted">(() =>
  thermal.status === "active" ? "danger" : thermal.status === "monitoring" ? "success" : "muted",
);

function patchCpu(p: Partial<typeof cpu.value>) {
  Object.assign(cpu.value, p);
  config.markDirty();
}
function patchGpu(p: Partial<typeof gpu.value>) {
  Object.assign(gpu.value, p);
  config.markDirty();
}

function onDefaultFps(v: number | null) {
  if (v === null) return;
  config.config.default_fps = v;
  config.markDirty();
}

const hidBackendOptions = [
  { label: "HIDRAW (recommended)", value: "hidraw" },
  { label: "LibUSB (direct)", value: "rusb" },
];

function onHidBackend(v: "hidraw" | "rusb") {
  config.config.hid_backend = v;
  config.markDirty();
}
</script>

<template>
  <div class="page settings-page">
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">Configuration</h2>
      </div>
      <div class="kv"><span class="muted">HID Backend</span>
        <n-select
          :value="config.config.hid_backend"
          :options="hidBackendOptions"
          size="small"
          style="width: 220px"
          @update:value="onHidBackend"
        />
      </div>
      <div class="kv">
        <span class="muted">Turn off LCDs on shutdown</span>
        <n-switch
          :value="config.config.turn_off_lcds_on_shutdown"
          aria-label="Turn off LCDs on shutdown"
          @update:value="(v: boolean) => { config.config.turn_off_lcds_on_shutdown = v; config.markDirty(); }"
        />
      </div>
      <div class="kv"><span class="muted">LCD count</span><span>{{ config.config.lcds.length }}</span></div>
      <div class="kv"><span class="muted">Fan curve count</span><span>{{ config.config.fan_curves.length }}</span></div>
      <div class="kv"><span class="muted">Max FPS Limit</span>
        <n-input-number :value="config.config.default_fps" :min="1" :max="120" size="small" @update:value="onDefaultFps" />
      </div>
      <div class="kv">
        <span class="muted">Hardware video acceleration</span>
        <n-switch
          :value="config.config.hardware_video"
          :disabled="!daemon.info?.capabilities.includes('hardware_video')"
          aria-label="Hardware video acceleration"
          @update:value="(v: boolean) => { config.config.hardware_video = v; config.markDirty(); }"
        />
      </div>
      <p class="hint">Uses available GPU video encoders and decoders, with software fallback. Save to apply to active streams.</p>
      <p v-if="daemon.connected && !daemon.info?.capabilities.includes('hardware_video')" class="hint">Update the daemon to manage hardware video from Settings.</p>
    </section>

    <ServiceStatus />
    <StateBackups />
    <CatalogStorage />
    <CatalogStorage managed />

    <section class="card">
      <div class="section-head">
        <h2 class="section-title">Daemon Status</h2>
        <span class="status-tag">
          <StatusDot :color="daemon.connected ? 'success' : 'danger'" />
          {{ daemon.connected ? "Connected" : "Offline" }}
        </span>
      </div>
      <div class="kv"><span class="muted">Socket</span><span class="mono">{{ daemon.socketPath || "—" }}</span></div>
      <div class="kv"><span class="muted">Daemon version</span><span>{{ daemon.version || "Unavailable" }}</span></div>
      <template v-if="daemon.info">
        <div class="kv"><span class="muted">Configuration mode</span><span>{{ daemon.info.mode }}</span></div>
        <div class="kv"><span class="muted">Configuration file</span><span class="mono">{{ daemon.info.config_path }}</span></div>
      </template>
    </section>

    <!-- Thermal alert -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">Thermal Alert</h2>
        <span class="status-tag" :class="{ 'pulse-danger': thermal.status === 'active' }">
          <StatusDot :color="thermalStatusColor" :pulse="thermal.status === 'active'" />
          {{ thermalStatusText }}
        </span>
      </div>
      <p class="hint">
        When CPU/GPU temperature exceeds the threshold, all RGB devices switch to the alert color
        until temperature drops below the threshold.
      </p>

      <div class="thermal-grid">
        <div class="source">
          <div class="source-head">CPU</div>
          <n-checkbox :checked="cpu.enabled" @update:checked="(v) => patchCpu({ enabled: v })">Enable</n-checkbox>
          <div class="field">
            <label class="muted">Threshold (°C)</label>
            <n-input-number :value="cpu.threshold" :min="20" :max="120" size="small" :disabled="!cpu.enabled" @update:value="(v) => patchCpu({ threshold: v ?? 80 })" />
          </div>
          <ColorPicker label="Alert color" :model-value="cpu.alert_color" :disabled="!cpu.enabled" @update:model-value="(v: any) => patchCpu({ alert_color: v })" />
        </div>
        <div class="source">
          <div class="source-head">GPU</div>
          <n-checkbox :checked="gpu.enabled" @update:checked="(v) => patchGpu({ enabled: v })">Enable</n-checkbox>
          <div class="field">
            <label class="muted">Threshold (°C)</label>
            <n-input-number :value="gpu.threshold" :min="20" :max="120" size="small" :disabled="!gpu.enabled" @update:value="(v) => patchGpu({ threshold: v ?? 80 })" />
          </div>
          <ColorPicker label="Alert color" :model-value="gpu.alert_color" :disabled="!gpu.enabled" @update:model-value="(v: any) => patchGpu({ alert_color: v })" />
        </div>
      </div>
    </section>

    <!-- OpenRGB -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">OpenRGB</h2>
        <span class="status-tag"><StatusDot :color="openrgbDot" />{{ openrgbStatus }}</span>
      </div>
      <n-checkbox v-model:checked="openrgbEnabled" style="margin-top: 1rem;">Enable OpenRGB SDK server</n-checkbox>
      <OpenRgbRegionsToggle v-if="openrgbEnabled" />
      <n-alert v-if="daemon.connected && daemon.openrgbEnabled && daemon.openrgbError" type="error" style="margin-top: 1rem;">{{ daemon.openrgbError }}</n-alert>
      <n-button v-if="daemon.connected && daemon.openrgbEnabled && daemon.openrgbError && !daemon.openrgbRunning" :loading="retryingOpenRgb" :disabled="retryingOpenRgb || !daemon.canWrite || !daemon.info?.capabilities.includes('openrgb_retry')" @click="retryOpenRgb">Retry OpenRGB</n-button>
      <p v-if="daemon.openrgbError && daemon.connected" class="hint">Retry uses the saved port. Save to apply edits.</p>
      <p v-if="openrgbFeedback" class="hint">{{ openrgbFeedback }}</p>
      <div class="field" v-if="openrgbEnabled">
        <label class="muted">Port</label>
        <n-input-number v-model:value="openrgbPort" :min="1" :max="65535" size="small" />
      </div>
      <p class="hint" v-if="openrgbEnabled">
        The server binds to a TCP port without authentication. While enabled, the daemon will not apply its own RGB effects.
      </p>
    </section>

    <!-- RGB Drift Detection -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">RGB Drift Detection</h2>
        <span class="status-tag">
          <StatusDot :color="config.config.rgb_drift_detection_enabled ? 'success' : 'muted'" />
          {{ config.config.rgb_drift_detection_enabled ? "Active" : "Disabled" }}
        </span>
      </div>
      <p class="hint">
        Re-applies saved RGB when a wireless device's firmware resets its lighting. Wireless devices only.
      </p>
      <n-checkbox
        style="margin-top: 1rem;"
        :checked="config.config.rgb_drift_detection_enabled"
        @update:checked="(v: boolean) => { config.config.rgb_drift_detection_enabled = v; config.markDirty(); }"
      >Enable drift detection</n-checkbox>
      <div class="field" v-if="config.config.rgb_drift_detection_enabled">
        <label class="muted">Check interval (ms)</label>
        <n-input-number
          :value="config.config.rgb_drift_detection_interval_ms"
          :min="100"
          :max="10000"
          :step="100"
          size="small"
          @update:value="(v: number | null) => { config.config.rgb_drift_detection_interval_ms = v ?? 1000; config.markDirty(); }"
        />
      </div>
    </section>

    <!-- About -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">About</h2>
      </div>
      <p class="about-line">
        Open-source Linux replacement for L-Connect 3
        <span class="muted"> · v{{ appVersion }}</span>
      </p>
      <button class="repo-link" @click="openUrl(REPO_URL)">
        <ExternalLink :size="13" /> github.com/sgtaziz/lian-li-linux
      </button>
    </section>
  </div>
</template>

<style scoped>
.settings-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 800px;
}
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}
.section-head .section-title {
  margin: 0;
}
.status-tag {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  white-space: nowrap;
}
.kv {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-1) 0;
  font-size: var(--font-size-sm);
  margin-top: var(--space-2);
}
.mono {
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
}
.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  margin-top: var(--space-2);
}
.hint {
  color: var(--text-muted);
  font-size: var(--font-size-sm);
  margin: var(--space-2) 0 0;
}
.thermal-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-4);
  margin-top: var(--space-3);
}
.source {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.source-head {
  font-weight: 600;
}
.about-line {
  margin: 0;
  margin-top: 1rem;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1);
}
.repo-link {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  background: none;
  border: none;
  padding: 0;
  padding-top: 1rem;
  color: var(--accent);
  cursor: pointer;
}
.repo-link:hover {
  color: var(--accent-hover);
}
</style>
