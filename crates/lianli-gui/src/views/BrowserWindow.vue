<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RefreshCw, Download, CheckCircle, AlertCircle, Loader2, X, ExternalLink } from "lucide-vue-next";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useExternalLink } from "@/composables/useExternalLink";
import { getVersion } from "@tauri-apps/api/app";
import type { CatalogManifest, CatalogTemplate, DeviceInfo, LcdTemplate, SensorBindings, SensorCategory, SensorInfo } from "@/types";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import { screenPresets } from "@/constants/screen";
import { useConfigStore } from "@/stores/config";
import { useLcdStore, LCD_TEMPLATES_CHANGED_EVENT, type CatalogInstallStatus } from "@/stores/lcd";
import { emit } from "@tauri-apps/api/event";
import { boundedFetch } from "@/utils/boundedFetch";
import { backendText } from "@/utils/backendText";
import { useTranslate } from "@/i18n";
import { useIpc } from "@/composables/useIpc";
import { isSensorCategory } from "@/stores/sensorOptions";
import SensorBindingDialog, { type SensorBindingRequest } from "@/components/editor/SensorBindingDialog.vue";

const config = useConfigStore();
const lcd = useLcdStore();
const openExternal = useExternalLink();
const ipc = useIpc();
const t = useTranslate();

const CATALOG_URL =
  "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates/default_templates.json";
const ASSET_BASE =
  "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates/assets";

const loading = ref(false);
const error = ref("");
const templates = ref<CatalogTemplate[]>([]);
const previewCache = ref<Record<string, string>>({});
const previewErrors = ref<Record<string, string>>({});
const previewOpen = ref(false);
const selectedPreview = ref<CatalogTemplate | null>(null);
let downloads = new AbortController();
let disposed = false;
function clearPreviews() {
  previewOpen.value = false;
  selectedPreview.value = null;
  for (const url of Object.values(previewCache.value)) URL.revokeObjectURL(url);
  previewCache.value = {};
  previewErrors.value = {};
}
onUnmounted(() => { disposed = true; downloads.abort(); clearPreviews(); });
const installState = ref<Record<string, "idle" | "installing" | "installed" | "error">>({});

const installedIds = ref<Set<string>>(new Set());
const monitoringInstall = ref(false);
const startingInstall = ref(false);
const installProgress = ref("");
const preparingId = ref("");
const bindingRequest = ref<SensorBindingRequest | null>(null);
let bindingEntry: CatalogTemplate | null = null;

type RotationFilter = "all" | "upright" | "rotated";

const screens = ref<DeviceInfo[]>([]);
const sizeFilter = ref<string | null>(null);
const rotationFilter = ref<RotationFilter>("all");

function sizeKey(width = 0, height = 0): string {
  return `${Math.max(width, height)}x${Math.min(width, height)}`;
}

const sizeOptions = computed(() => {
  const keys = new Set(templates.value.filter((tpl) => tpl.base_width && tpl.base_height).map((tpl) => sizeKey(tpl.base_width, tpl.base_height)));
  return [...keys].sort().map((key) => {
    const [long, short] = key.split("x");
    const owners = [
      ...screenPresets
        .filter((preset) => sizeKey(preset.width, preset.height) === key)
        .flatMap((preset) => preset.label.replace(/\s*\(.*\)$/, "").split(" / ")),
      ...screens.value
        .filter((device) => device.screen_width && device.screen_height && sizeKey(device.screen_width, device.screen_height) === key)
        .map((device) => device.name),
    ];
    const size = `${long}×${short}`;
    return { value: key, label: owners.length ? t("editor.browser.sizeForDevices", { size, devices: [...new Set(owners)].join(", ") }) : size };
  });
});

const rotationOptions = computed(() => [
  { value: "all" as const, label: t("editor.browser.rotationAll") },
  { value: "upright" as const, label: t("editor.browser.rotationUpright") },
  { value: "rotated" as const, label: t("editor.browser.rotated") },
]);

const visibleTemplates = computed(() =>
  templates.value.filter((tpl) =>
    (!sizeFilter.value || sizeKey(tpl.base_width, tpl.base_height) === sizeFilter.value)
    && (rotationFilter.value === "all" || !!tpl.rotated === (rotationFilter.value === "rotated")),
  ),
);

const selectedVariant = ref<Record<string, string>>({});

const cards = computed(() => {
  const groups = new Map<string, CatalogTemplate[]>();
  for (const tpl of visibleTemplates.value) {
    const key = tpl.group || tpl.id;
    groups.set(key, [...(groups.get(key) ?? []), tpl]);
  }
  return [...groups].map(([key, variants]) => ({
    key,
    variants,
    tpl: variants.find((variant) => variant.id === selectedVariant.value[key]) ?? variants[0],
  }));
});

async function loadScreens() {
  try {
    const devices = await ipc.request<DeviceInfo[]>("ListDevices");
    screens.value = devices.filter((device) => device.has_lcd);
  } catch {
    screens.value = [];
  }
}

onMounted(async () => {
  void loadScreens();
  await config.load().catch(() => undefined);
  for (const t of config.templates) installedIds.value.add(t.id);
  if (!disposed) void checkInstall();
  await fetchCatalog();
});

async function fetchCatalog() {
  if (loading.value || disposed) return;
  downloads.abort();
  downloads = new AbortController();
  const controller = downloads;
  clearPreviews();
  loading.value = true;
  error.value = "";
  try {
    const blob = await boundedFetch(CATALOG_URL, 1024 * 1024, controller.signal);
    const manifest: CatalogManifest = JSON.parse(await blob.text());
    if (manifest.schema_version !== 1) {
      throw new Error(t("editor.browser.unsupportedSchema", { version: manifest.schema_version }));
    }
    if (!Array.isArray(manifest.templates) || manifest.templates.length > 128
      || manifest.templates.some((t) => !t || [t.id, t.name, t.folder, t.preview, t.min_daemon_version].some((value) => typeof value !== "string" || value.length > 512))
      || manifest.templates.some((t) => [t.group, t.variant].some((value) => value !== undefined && (typeof value !== "string" || value.length > 64)))
      || new Set(manifest.templates.map((t) => t.id)).size !== manifest.templates.length) {
      throw new Error(t("editor.browser.invalidCatalog"));
    }
    const ver = await getVersion().catch(() => null);
    if (controller.signal.aborted) return;
    templates.value = ver
      ? manifest.templates.filter((t) => versionGte(ver, t.min_daemon_version))
      : manifest.templates;
    void loadPreviews(templates.value, controller);
  } catch (e) {
    if (!controller.signal.aborted) error.value = t("editor.browser.catalogError", { error: String(e) });
  } finally {
    loading.value = false;
  }
}

function versionGte(have: string, need: string): boolean {
  const parse = (s: string) =>
    s
      .replace(/^v/, "")
      .split(".")
      .map((p) => parseInt(p.replace(/[^0-9].*/, ""), 10) || 0);
  const [hh = 0, hm = 0, hp = 0] = parse(have);
  const [nh = 0, nm = 0, np = 0] = parse(need);
  return hh > nh || (hh === nh && (hm > nm || (hm === nm && hp >= np)));
}

async function loadPreviews(items: CatalogTemplate[], controller: AbortController) {
  let next = 0;
  let cachedBytes = 0;
  await Promise.all(Array.from({ length: 4 }, async () => {
    while (!controller.signal.aborted && next < items.length) {
      const item = items[next++];
      try {
        if (cachedBytes >= 16 * 1024 * 1024) throw new Error(t("editor.browser.previewCacheFull"));
        const blob = await boundedFetch(`${ASSET_BASE}/${item.folder}/${item.preview}`, 1024 * 1024, controller.signal);
        if (controller.signal.aborted) return;
        if (cachedBytes + blob.size > 16 * 1024 * 1024) throw new Error(t("editor.browser.previewCacheFull"));
        cachedBytes += blob.size;
        previewCache.value[item.id] = URL.createObjectURL(blob);
      } catch (e) {
        if (!controller.signal.aborted) previewErrors.value[item.id] = t("editor.browser.previewError", { error: String(e) });
      }
    }
  }));
}

async function install(entry: CatalogTemplate) {
  if (startingInstall.value || monitoringInstall.value || bindingRequest.value || installState.value[entry.id] === "installing") return;
  startingInstall.value = true;
  preparingId.value = entry.id;
  try {
    const request = await sensorBindingRequest(entry, downloads.signal);
    if (request) {
      bindingEntry = entry;
      bindingRequest.value = request;
      return;
    }
  } catch (e) {
    if (!downloads.signal.aborted) installProgress.value = t("editor.browser.sensorsUnavailable", { error: String(e) });
    return;
  } finally {
    startingInstall.value = false;
    preparingId.value = "";
  }
  await startInstall(entry, {});
}

async function sensorBindingRequest(entry: CatalogTemplate, signal: AbortSignal): Promise<SensorBindingRequest | null> {
  const blob = await boundedFetch(`${ASSET_BASE}/${entry.folder}/${entry.template_file}`, 1024 * 1024, signal);
  const bytes = await blob.arrayBuffer();
  if (await sha256Hex(bytes) !== entry.template_sha256.toLowerCase()) throw new Error(t("editor.browser.templateChecksum"));
  const template = JSON.parse(new TextDecoder().decode(bytes)) as LcdTemplate;
  const categories = new Set<SensorCategory>();
  for (const widget of template.widgets ?? []) {
    if (widget?.kind?.source && isSensorCategory(widget.sensor_category)) categories.add(widget.sensor_category);
  }
  if (!categories.size) return null;
  const ordered = [...categories];
  const [sensors, suggestions] = await Promise.all([
    ipc.request<SensorInfo[]>("ListSensors"),
    lcd.suggestSensorSources(ordered),
  ]);
  signal.throwIfAborted();
  return {
    templateName: entry.name,
    sensors,
    rows: ordered.map((category, index) => ({ category, suggested: suggestions[index] ?? null })),
  };
}

async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function onBindingsConfirmed(bindings: SensorBindings) {
  const entry = bindingEntry;
  closeBindings();
  if (entry) void startInstall(entry, bindings);
}

function closeBindings() {
  bindingRequest.value = null;
  bindingEntry = null;
}

async function startInstall(entry: CatalogTemplate, bindings: SensorBindings) {
  if (startingInstall.value || monitoringInstall.value) return;
  startingInstall.value = true;
  installState.value[entry.id] = "installing";
  try {
    await monitorInstall(await lcd.installTemplate(entry, bindings));
  } catch (e) {
    installState.value[entry.id] = "error";
    error.value = t("editor.browser.installUnconfirmedMayFinish", { error: String(e) });
  } finally {
    startingInstall.value = false;
  }
}

async function checkInstall() {
  if (monitoringInstall.value || disposed) return;
  try {
    const status = await lcd.catalogInstallStatus();
    if (status) await monitorInstall(status);
    else installProgress.value = t("editor.browser.noInstallRecorded");
  } catch (e) {
    installProgress.value = t("editor.browser.statusUnavailable", { error: String(e) });
  }
}

async function monitorInstall(initial: CatalogInstallStatus) {
  if (monitoringInstall.value || disposed) return;
  monitoringInstall.value = true;
  let status = initial;
  try {
    while (!disposed) {
      installState.value[status.template_id] = status.finished
        ? (status.error ? "error" : "installed") : "installing";
      if (status.finished) {
        if (status.error) throw new Error(backendText(status.error, status.error_ui));
        installProgress.value = t("editor.browser.installSaved", { id: status.template_id });
        await emit(LCD_TEMPLATES_CHANGED_EVENT);
        await config.load();
        installedIds.value = new Set(config.templates.map((template) => template.id));
        return;
      }
      installProgress.value = t("editor.browser.installing", { id: status.template_id });
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (disposed) return;
      const next = await lcd.catalogInstallStatus();
      if (!next || next.operation_id !== initial.operation_id) {
        throw new Error(t("editor.browser.installChanged"));
      }
      status = next;
    }
  } catch (e) {
    installState.value[initial.template_id] = "error";
    installProgress.value = t("editor.browser.installUnconfirmed", { error: String(e) });
  } finally {
    monitoringInstall.value = false;
  }
}

function closeWindow() {
  void getCurrentWebviewWindow().close();
}

const PUBLISHING_URL = "https://github.com/sgtaziz/lian-li-linux/tree/main/templates";

function openPreview(entry: CatalogTemplate) {
  if (!previewCache.value[entry.id]) return;
  selectedPreview.value = entry;
  previewOpen.value = true;
}

/** Effective (post-rotation) aspect ratio for a catalog template. */
function previewAspect(t: CatalogTemplate): string {
  const w = t.rotated ? t.base_height : t.base_width;
  const h = t.rotated ? t.base_width : t.base_height;
  return w && h ? `${w} / ${h}` : "1 / 1";
}
</script>

<template>
  <div class="browser-window">
    <div class="topbar">
      <span class="title">{{ t("editor.browser.title") }}</span>
      <button class="guide" :title="t('editor.browser.publishingGuideTooltip')" @click="openExternal(PUBLISHING_URL)">
        <ExternalLink :size="13" /> {{ t("editor.browser.publishingGuide") }}
      </button>
      <div class="spacer" />
      <n-button size="small" quaternary :disabled="monitoringInstall" @click="checkInstall">{{ t("editor.browser.checkInstall") }}</n-button>
      <n-button size="small" quaternary :loading="loading" @click="fetchCatalog">
        <template #icon><RefreshCw :size="14" /></template>
        {{ t("common.refresh") }}
      </n-button>
      <n-button size="small" quaternary @click="closeWindow"><template #icon><X :size="14" /></template>{{ t("common.close") }}</n-button>
    </div>

    <div class="content">
      <p v-if="installProgress" role="status">{{ installProgress }}</p>
      <div v-if="loading" class="state">
        <Loader2 :size="28" class="spin" />
        <span>{{ t("editor.browser.loadingCatalog") }}</span>
      </div>

      <div v-else-if="error" class="state error">
        <AlertCircle :size="28" />
        <span>{{ error }}</span>
      </div>

      <div v-else-if="!templates.length" class="state muted">
        {{ t("editor.browser.noTemplates") }}
      </div>

      <template v-else>
      <div class="filters">
        <n-select
          size="small"
          class="size-filter"
          clearable
          :value="sizeFilter"
          :options="sizeOptions"
          :consistent-menu-width="false"
          :placeholder="t('editor.browser.allScreens')"
          :aria-label="t('editor.browser.screenFilter')"
          @update:value="(v: string | null) => (sizeFilter = v)"
        />
        <SegmentedControl
          size="small"
          :model-value="rotationFilter"
          :options="rotationOptions"
          :label="t('editor.browser.rotationFilter')"
          @update:model-value="(v: RotationFilter) => (rotationFilter = v)"
        />
      </div>
      <div v-if="!visibleTemplates.length" class="state muted">{{ t("editor.browser.noMatches") }}</div>
      <div v-else class="grid">
        <div v-for="{ key, tpl, variants } in cards" :key="key" class="card tpl-card">
          <button
            type="button"
            class="preview"
            :style="{ aspectRatio: previewAspect(tpl) }"
            :disabled="!previewCache[tpl.id]"
            :aria-label="t('editor.browser.enlargePreviewOf', { name: tpl.name })"
            :title="t('editor.browser.enlargePreview')"
            @click="openPreview(tpl)"
          >
            <img v-if="previewCache[tpl.id]" :src="previewCache[tpl.id]" alt="" />
            <span v-else class="preview-ph" :title="previewErrors[tpl.id]">{{ previewErrors[tpl.id] ? t("editor.browser.previewUnavailable") : "" }}</span>
          </button>
          <div class="info">
            <div class="name">{{ tpl.name }}</div>
            <div class="author muted" v-if="tpl.author">{{ t("editor.browser.byAuthor", { author: tpl.author }) }}</div>
            <div class="badges">
              <span class="badge" v-if="tpl.base_width">{{ tpl.base_width }}×{{ tpl.base_height }}</span>
              <span class="badge" v-if="tpl.rotated">{{ t("editor.browser.rotated") }}</span>
            </div>
            <div class="desc muted">{{ tpl.description }}</div>
            <div v-if="variants.length > 1" class="variants" role="group" :aria-label="t('editor.browser.variants')">
              <button
                v-for="variant in variants"
                :key="variant.id"
                type="button"
                class="variant"
                :class="{ selected: variant.id === tpl.id }"
                :aria-pressed="variant.id === tpl.id"
                @click="selectedVariant[key] = variant.id"
              >
                <CheckCircle v-if="installedIds.has(variant.id)" :size="12" />{{ variant.variant || variant.name }}
              </button>
            </div>
            <n-button
              size="small"
              type="primary"
              :disabled="installedIds.has(tpl.id) || startingInstall || monitoringInstall"
              :loading="installState[tpl.id] === 'installing' || preparingId === tpl.id"
              @click="install(tpl)"
            >
              <template v-if="installState[tpl.id] === 'installing'" #icon><Loader2 :size="14" class="spin" /></template>
              <template v-else-if="installState[tpl.id] === 'installed' || installedIds.has(tpl.id)" #icon><CheckCircle :size="14" /></template>
              <template v-else #icon><Download :size="14" /></template>
              {{ installedIds.has(tpl.id) ? t("editor.browser.installed") : t("editor.browser.install") }}
            </n-button>
          </div>
        </div>
      </div>
      </template>
    </div>
    <n-modal
      v-model:show="previewOpen"
      preset="card"
      closable
      :title="selectedPreview?.name ?? t('editor.browser.templatePreview')"
      style="width: min(1400px, 95vw)"
    >
      <img
        v-if="selectedPreview && previewCache[selectedPreview.id]"
        class="enlarged-preview"
        :src="previewCache[selectedPreview.id]"
        :alt="t('editor.browser.previewOf', { name: selectedPreview.name })"
      />
    </n-modal>
    <SensorBindingDialog :request="bindingRequest" @confirm="onBindingsConfirmed" @cancel="closeBindings" />

  </div>
</template>

<style scoped>
.browser-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  width: 100vw;
  background: var(--md-surface);
}
.topbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
  padding: 20px 28px 12px;
}
.title {
  font-family: var(--font-display);
  font-size: 28px;
  font-weight: 600;
  letter-spacing: -0.3px;
}
.guide {
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  background: none;
  border: none;
  color: var(--md-primary);
  cursor: pointer;
  padding: 6px 12px;
  border-radius: 16px;
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}
.guide:hover {
  background: color-mix(in srgb, var(--md-primary) 10%, transparent);
}
.guide:focus-visible {
  outline: 2px solid var(--md-primary);
}
.spacer {
  flex: 1;
}
.content {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-1) 28px 28px;
}
.state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-8);
  color: var(--md-on-surface-variant);
}
.state.error {
  color: var(--danger);
}
.filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}
.size-filter {
  width: min(360px, 100%);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: var(--space-4);
}
.tpl-card {
  padding: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 24px;
  border: 1px solid var(--md-outline-variant);
}
.info {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  flex: 1;
  padding: var(--space-3) var(--space-4) var(--space-4);
}
.info .n-button {
  align-self: flex-start;
  margin-top: auto;
}
.preview {
  display: block;
  padding: 0;
  width: 100%;
  max-height: 200px;
  /* aspect-ratio is set inline from each template's effective dimensions. */
  background: #050507;
  overflow: hidden;
  border: 0;
}
.preview:not(:disabled) {
  cursor: zoom-in;
}
.preview:disabled {
  cursor: default;
}
.preview:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -3px;
}
.enlarged-preview {
  display: block;
  width: 100%;
  max-height: 70vh;
  object-fit: contain;
}
.preview img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.preview-ph {
  display: block;
  width: 100%;
  height: 100%;
}
.name {
  font-family: var(--font-display);
  font-weight: 600;
  font-size: 17px;
}
.author {
  font-size: var(--font-size-xs);
}
.badges {
  display: flex;
  gap: var(--space-1);
  flex-wrap: wrap;
}
.badge {
  font-size: 11px;
  font-weight: 700;
  background: var(--md-secondary-container);
  padding: 3px 9px;
  border-radius: 8px;
  color: var(--md-on-secondary-container);
}
.variants {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.variant {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  border: 1px solid var(--md-outline-variant);
  border-radius: 10px;
  background: none;
  color: var(--md-on-surface-variant);
  font: inherit;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}
.variant.selected {
  border-color: var(--md-primary);
  background: color-mix(in srgb, var(--md-primary) 16%, transparent);
  color: var(--md-on-surface);
}
.variant:focus-visible {
  outline: 2px solid var(--md-primary);
}
.desc {
  font-size: 13px;
  line-height: 1.45;
  min-height: 28px;
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
