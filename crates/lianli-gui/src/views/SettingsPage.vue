<script setup lang="ts">
import { computed } from "vue";
import { ExternalLink } from "lucide-vue-next";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import StatusDot from "@/components/common/StatusDot.vue";
import ColorPicker from "@/components/rgb/ColorPicker.vue";

const REPO_URL = "https://github.com/sgtaziz/lian-li-linux";
const daemon = useDaemonStore();
const config = useConfigStore();
const thermal = useThermalStore();

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
  if (!openrgbEnabled.value) return "Disabled";
  if (daemon.openrgbError) return "Error";
  if (daemon.openrgbRunning) return `Port ${daemon.openrgbPort ?? rgb.value.openrgb_port}`;
  return "Starting…";
});
const openrgbDot = computed<"danger" | "success" | "warning" | "muted">(() =>
  !openrgbEnabled.value
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
</script>

<template>
  <div class="page settings-page">
    <!-- Daemon status -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">Daemon Status</h2>
        <span class="status-tag">
          <StatusDot :color="daemon.connected ? 'success' : 'danger'" />
          {{ daemon.connected ? "Connected" : "Offline" }}
        </span>
      </div>
      <div class="kv"><span class="muted">Socket</span><span class="mono">{{ daemon.socketPath || "—" }}</span></div>
    </section>

    <!-- Configuration -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">Configuration</h2>
      </div>
      <div class="kv"><span class="muted">LCD count</span><span>{{ config.config.lcds.length }}</span></div>
      <div class="kv"><span class="muted">Fan curve count</span><span>{{ config.config.fan_curves.length }}</span></div>
      <div class="kv"><span class="muted">Default FPS</span>
        <n-input-number :value="config.config.default_fps" :min="1" :max="60" size="small" @update:value="onDefaultFps" />
      </div>
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
        <span class="muted"> · v0.6.1</span>
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
