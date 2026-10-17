<script setup lang="ts">
import { computed, ref, onMounted, watch } from "vue";
import OpenRgbOptions from "@/components/rgb/OpenRgbOptions.vue";
import { backendText, invoke } from "@/utils/backendText";
import { ExternalLink } from "lucide-vue-next";
import { useExternalLink } from "@/composables/useExternalLink";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useThermalStore } from "@/stores/thermal";
import StatusDot from "@/components/common/StatusDot.vue";
import ColorPicker from "@/components/rgb/ColorPicker.vue";
import ServiceStatus from "@/components/common/ServiceStatus.vue";
import AppUpdates from "@/components/common/AppUpdates.vue";
import StateBackups from "@/components/common/StateBackups.vue";
import CatalogStorage from "@/components/common/CatalogStorage.vue";
import { useIpc } from "@/composables/useIpc";
import { DAEMON_MODE_LABELS } from "@/utils/serviceSetup";
import { useTranslate } from "@/i18n";
import { useLanguageStore, type LanguagePreference } from "@/stores/language";

const t = useTranslate();
const language = useLanguageStore();
const languageOptions = computed(() => [
  { label: t("language.system"), value: "system" },
  { label: "English", value: "en" },
  { label: "简体中文", value: "zh-CN" },
]);

const REPO_URL = "https://github.com/sgtaziz/lian-li-linux";
const daemon = useDaemonStore();
const config = useConfigStore();
const thermal = useThermalStore();
const ipc = useIpc();
const openExternal = useExternalLink();
const retryingOpenRgb = ref(false);
const openrgbRetryQueued = ref(false);
const openrgbFeedback = ref("");
watch(() => daemon.info?.instance_id, () => { openrgbFeedback.value = ""; openrgbRetryQueued.value = false; });
async function retryOpenRgb() {
  if (retryingOpenRgb.value || !daemon.canWrite) return;
  const instance = daemon.info?.instance_id;
  retryingOpenRgb.value = true;
  openrgbFeedback.value = "";
  openrgbRetryQueued.value = false;
  try {
    await ipc.request("RetryOpenRgb");
    if (daemon.info?.instance_id !== instance) return;
    openrgbRetryQueued.value = true;
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
  if (!daemon.connected) return t("settings.openrgb.disconnected");
  if (!daemon.openrgbEnabled) return t("common.disabled");
  if (daemon.openrgbError) return t("common.error");
  if (daemon.openrgbRunning) {
    return daemon.openrgbPort === null ? t("settings.openrgb.running") : t("settings.openrgb.port", { port: daemon.openrgbPort });
  }
  return t("settings.openrgb.starting");
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
      return t("settings.thermal.active");
    case "monitoring":
      return t("settings.thermal.monitoring");
    default:
      return t("common.disabled");
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

const hidBackendOptions = computed(() => [
  { label: t("settings.configuration.hidraw"), value: "hidraw" },
  { label: t("settings.configuration.libusb"), value: "rusb" },
]);

function onHidBackend(v: "hidraw" | "rusb") {
  config.config.hid_backend = v;
  config.markDirty();
}
</script>

<template>
  <div class="page settings-page">
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">{{ t("language.title") }}</h2>
      </div>
      <div class="kv"><span class="muted">{{ t("language.label") }}</span>
        <n-select
          :value="language.preference"
          :options="languageOptions"
          size="small"
          style="width: 220px"
          @update:value="(v: LanguagePreference) => language.set(v)"
        />
      </div>
      <p class="hint">{{ t("language.hint") }}</p>
    </section>

    <section class="card">
      <div class="section-head">
        <h2 class="section-title">{{ t("settings.configuration.title") }}</h2>
      </div>
      <div class="kv"><span class="muted">{{ t("settings.configuration.hidBackend") }}</span>
        <n-select
          :value="config.config.hid_backend"
          :options="hidBackendOptions"
          size="small"
          style="width: 220px"
          @update:value="onHidBackend"
        />
      </div>
      <div class="kv">
        <span class="muted">{{ t("settings.configuration.turnOffLcds") }}</span>
        <n-switch
          :value="config.config.turn_off_lcds_on_shutdown"
          :aria-label="t('settings.configuration.turnOffLcds')"
          @update:value="(v: boolean) => { config.config.turn_off_lcds_on_shutdown = v; config.markDirty(); }"
        />
      </div>
      <div class="kv"><span class="muted">{{ t("settings.configuration.lcdCount") }}</span><span>{{ config.config.lcds.length }}</span></div>
      <div class="kv"><span class="muted">{{ t("settings.configuration.fanCurveCount") }}</span><span>{{ config.config.fan_curves.length }}</span></div>
      <div class="kv"><span class="muted">{{ t("settings.configuration.maxFps") }}</span>
        <n-input-number :value="config.config.default_fps" :min="1" :max="120" size="small" @update:value="onDefaultFps" />
      </div>
      <div class="kv">
        <span class="muted">{{ t("settings.configuration.hardwareVideo") }}</span>
        <n-switch
          :value="config.config.hardware_video"
          :disabled="!daemon.info?.capabilities.includes('hardware_video')"
          :aria-label="t('settings.configuration.hardwareVideo')"
          @update:value="(v: boolean) => { config.config.hardware_video = v; config.markDirty(); }"
        />
      </div>
      <p class="hint">{{ t("settings.configuration.hardwareVideoHint") }}</p>
      <p v-if="daemon.connected && !daemon.info?.capabilities.includes('hardware_video')" class="hint">{{ t("settings.configuration.hardwareVideoUpdate") }}</p>
    </section>

    <AppUpdates />
    <ServiceStatus />
    <StateBackups />
    <CatalogStorage />
    <CatalogStorage managed />

    <section class="card">
      <div class="section-head">
        <h2 class="section-title">{{ t("settings.daemon.title") }}</h2>
        <span class="status-tag">
          <StatusDot :color="daemon.connected ? 'success' : 'danger'" />
          {{ daemon.connected ? t("settings.daemon.connected") : t("common.offline") }}
        </span>
      </div>
      <div class="kv"><span class="muted">{{ t("settings.daemon.socket") }}</span><span class="mono">{{ daemon.socketPath || "—" }}</span></div>
      <div class="kv"><span class="muted">{{ t("settings.daemon.version") }}</span><span>{{ daemon.version || t("common.unavailable") }}</span></div>
      <template v-if="daemon.info">
        <div class="kv"><span class="muted">{{ t("settings.daemon.mode") }}</span><span>{{ t(DAEMON_MODE_LABELS[daemon.info.mode]) }}</span></div>
        <div class="kv"><span class="muted">{{ t("settings.daemon.file") }}</span><span class="mono">{{ daemon.info.config_path }}</span></div>
      </template>
    </section>

    <!-- Thermal alert -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">{{ t("settings.thermal.title") }}</h2>
        <span class="status-tag" :class="{ 'pulse-danger': thermal.status === 'active' }">
          <StatusDot :color="thermalStatusColor" :pulse="thermal.status === 'active'" />
          {{ thermalStatusText }}
        </span>
      </div>
      <p class="hint">{{ t("settings.thermal.hint") }}</p>

      <div class="thermal-grid">
        <div class="source">
          <div class="source-head">CPU</div>
          <n-checkbox :checked="cpu.enabled" @update:checked="(v) => patchCpu({ enabled: v })">{{ t("settings.thermal.enable") }}</n-checkbox>
          <div class="field">
            <label class="muted">{{ t("settings.thermal.threshold") }}</label>
            <n-input-number :value="cpu.threshold" :min="20" :max="120" size="small" :disabled="!cpu.enabled" @update:value="(v) => patchCpu({ threshold: v ?? 80 })" />
          </div>
          <ColorPicker :label="t('settings.thermal.alertColor')" :model-value="cpu.alert_color" :disabled="!cpu.enabled" @update:model-value="(v: any) => patchCpu({ alert_color: v })" />
        </div>
        <div class="source">
          <div class="source-head">GPU</div>
          <n-checkbox :checked="gpu.enabled" @update:checked="(v) => patchGpu({ enabled: v })">{{ t("settings.thermal.enable") }}</n-checkbox>
          <div class="field">
            <label class="muted">{{ t("settings.thermal.threshold") }}</label>
            <n-input-number :value="gpu.threshold" :min="20" :max="120" size="small" :disabled="!gpu.enabled" @update:value="(v) => patchGpu({ threshold: v ?? 80 })" />
          </div>
          <ColorPicker :label="t('settings.thermal.alertColor')" :model-value="gpu.alert_color" :disabled="!gpu.enabled" @update:model-value="(v: any) => patchGpu({ alert_color: v })" />
        </div>
      </div>
    </section>

    <!-- OpenRGB -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">OpenRGB</h2>
        <span class="status-tag"><StatusDot :color="openrgbDot" />{{ openrgbStatus }}</span>
      </div>
      <div class="openrgb-row">
        <n-checkbox v-model:checked="openrgbEnabled">{{ t("settings.openrgb.enableServer") }}</n-checkbox>
        <label v-if="openrgbEnabled" class="port-row">
          <span class="muted">{{ t("settings.openrgb.portLabel") }}</span>
          <n-input-number v-model:value="openrgbPort" :min="1" :max="65535" size="small" />
        </label>
      </div>
      <template v-if="openrgbEnabled">
        <p class="hint">{{ t("settings.openrgb.authWarning") }}</p>
        <OpenRgbOptions class="openrgb-options" />
      </template>
      <n-alert v-if="daemon.connected && daemon.openrgbEnabled && daemon.openrgbError" type="error" style="margin-top: 1rem;">{{ backendText(daemon.openrgbError, daemon.openrgbErrorUi) }}</n-alert>
      <n-button v-if="daemon.connected && daemon.openrgbEnabled && daemon.openrgbError && !daemon.openrgbRunning" :loading="retryingOpenRgb" :disabled="retryingOpenRgb || !daemon.canWrite || !daemon.info?.capabilities.includes('openrgb_retry')" @click="retryOpenRgb">{{ t("settings.openrgb.retry") }}</n-button>
      <p v-if="daemon.openrgbError && daemon.connected" class="hint">{{ t("settings.openrgb.retryHint") }}</p>
      <p v-if="openrgbRetryQueued" class="hint">{{ t("settings.openrgb.retryQueued") }}</p>
      <p v-if="openrgbFeedback" class="hint">{{ openrgbFeedback }}</p>
    </section>

    <!-- RGB Drift Detection -->
    <section class="card">
      <div class="section-head">
        <h2 class="section-title">{{ t("settings.drift.title") }}</h2>
        <span class="status-tag">
          <StatusDot :color="config.config.rgb_drift_detection_enabled ? 'success' : 'muted'" />
          {{ config.config.rgb_drift_detection_enabled ? t("settings.drift.active") : t("common.disabled") }}
        </span>
      </div>
      <p class="hint">{{ t("settings.drift.hint") }}</p>
      <n-checkbox
        style="margin-top: 1rem;"
        :checked="config.config.rgb_drift_detection_enabled"
        @update:checked="(v: boolean) => { config.config.rgb_drift_detection_enabled = v; config.markDirty(); }"
      >{{ t("settings.drift.enable") }}</n-checkbox>
      <div class="field" v-if="config.config.rgb_drift_detection_enabled">
        <label class="muted">{{ t("settings.drift.interval") }}</label>
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
        <h2 class="section-title">{{ t("settings.about.title") }}</h2>
      </div>
      <p class="about-line">
        {{ t("settings.about.tagline") }}
        <span class="muted"> · v{{ appVersion }}</span>
      </p>
      <button class="repo-link" @click="openExternal(REPO_URL)">
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
.openrgb-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-4);
  margin-top: var(--space-3);
}
.port-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
}
.port-row .n-input-number {
  width: 110px;
}
.openrgb-options {
  margin-top: var(--space-3);
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
