<script setup lang="ts">
import { onMounted, ref } from "vue";
import { RefreshCw, Download, CheckCircle, AlertCircle, Loader2, X, ExternalLink } from "lucide-vue-next";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import type { CatalogManifest, CatalogTemplate } from "@/types";
import { useConfigStore } from "@/stores/config";
import { useLcdStore } from "@/stores/lcd";

const config = useConfigStore();
const lcd = useLcdStore();

const CATALOG_URL =
  "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates/default_templates.json";
const ASSET_BASE =
  "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates/assets";

const loading = ref(false);
const error = ref("");
const templates = ref<CatalogTemplate[]>([]);
const previewCache = ref<Record<string, string>>({});
const installState = ref<Record<string, "idle" | "installing" | "installed" | "error">>({});

const installedIds = ref<Set<string>>(new Set());

onMounted(async () => {
  await config.load().catch(() => undefined);
  for (const t of config.templates) installedIds.value.add(t.id);
  await fetchCatalog();
});

async function fetchCatalog() {
  loading.value = true;
  error.value = "";
  try {
    const resp = await fetch(CATALOG_URL);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const manifest: CatalogManifest = await resp.json();
    if (manifest.schema_version !== 1) {
      throw new Error(`unsupported catalog schema version ${manifest.schema_version}`);
    }
    templates.value = manifest.templates;
    // Lazily load previews.
    for (const t of templates.value) {
      void loadPreview(t);
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function loadPreview(t: CatalogTemplate) {
  if (previewCache.value[t.id]) return;
  try {
    const url = `${ASSET_BASE}/${t.folder}/${t.preview}`;
    const resp = await fetch(url);
    if (!resp.ok) return;
    const blob = await resp.blob();
    const reader = new FileReader();
    reader.onload = () => {
      previewCache.value[t.id] = reader.result as string;
    };
    reader.readAsDataURL(blob);
  } catch {
    // leave empty
  }
}

async function install(t: CatalogTemplate) {
  installState.value[t.id] = "installing";
  try {
    // Fetch template.json + files, build a local LcdTemplate, and persist via
    // SetLcdTemplates. Asset paths are rewritten to absolute install paths.
    const tplJson = await (await fetch(`${ASSET_BASE}/${t.folder}/${t.template_file}`)).text();
    const template = JSON.parse(tplJson);

    // Resolve sensor_category hints against local sensors, if present.
    for (const w of template.widgets ?? []) {
      if (w.sensor_category) {
        const cat = w.sensor_category;
        const match = config.sensors.find((s) => sensorMatchesCategory(s, cat));
        if (match && w.kind?.source !== undefined) {
          w.kind.source = match.source;
        }
        delete w.sensor_category;
      }
    }

    const idx = config.templates.findIndex((x) => x.id === template.id);
    if (idx >= 0) config.templates[idx] = template;
    else config.templates.push(template);
    await lcd.setTemplates(config.templates);
    installedIds.value.add(template.id);
    installState.value[t.id] = "installed";
    await config.load();
  } catch (e) {
    installState.value[t.id] = "error";
    error.value = `Install failed: ${e}`;
  }
}

function sensorMatchesCategory(_s: any, _cat: string): boolean {
  // The daemon's picker does proper categorization; in the browser we accept
  // any sensor for the requested category as a best-effort default.
  return true;
}

function closeWindow() {
  void getCurrentWebviewWindow().close();
}

const PUBLISHING_URL = "https://github.com/sgtaziz/lian-li-linux/tree/main/templates";

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
      <span class="title">Template Browser</span>
      <button class="guide" title="Open publishing guide in your browser" @click="openUrl(PUBLISHING_URL)">
        <ExternalLink :size="13" /> Publishing Guide
      </button>
      <div class="spacer" />
      <n-button size="small" quaternary :loading="loading" @click="fetchCatalog">
        <template #icon><RefreshCw :size="14" /></template>
        Refresh
      </n-button>
      <n-button size="small" quaternary @click="closeWindow"><template #icon><X :size="14" /></template>Close</n-button>
    </div>

    <div class="content">
      <div v-if="loading" class="state">
        <Loader2 :size="28" class="spin" />
        <span>Loading catalog…</span>
      </div>

      <div v-else-if="error" class="state error">
        <AlertCircle :size="28" />
        <span>{{ error }}</span>
      </div>

      <div v-else-if="!templates.length" class="state muted">
        No templates available.
      </div>

      <div v-else class="grid">
        <div v-for="t in templates" :key="t.id" class="card tpl-card">
          <div class="preview" :style="{ aspectRatio: previewAspect(t) }">
            <img v-if="previewCache[t.id]" :src="previewCache[t.id]" alt="" />
            <div v-else class="preview-ph" />
          </div>
          <div class="info">
            <div class="name">{{ t.name }}</div>
            <div class="author muted" v-if="t.author">by {{ t.author }}</div>
            <div class="badges">
              <span class="badge" v-if="t.base_width">{{ t.base_width }}×{{ t.base_height }}</span>
              <span class="badge" v-if="t.rotated">Rotated</span>
            </div>
            <div class="desc muted">{{ t.description }}</div>
            <n-button
              size="small"
              type="primary"
              :disabled="installedIds.has(t.id)"
              :loading="installState[t.id] === 'installing'"
              @click="install(t)"
            >
              <template v-if="installState[t.id] === 'installing'" #icon><Loader2 :size="14" class="spin" /></template>
              <template v-else-if="installState[t.id] === 'installed' || installedIds.has(t.id)" #icon><CheckCircle :size="14" /></template>
              <template v-else #icon><Download :size="14" /></template>
              {{ installedIds.has(t.id) ? "Installed" : "Install" }}
            </n-button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.browser-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  width: 100vw;
}
.topbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
}
.title {
  font-weight: 600;
}
.guide {
  font-size: var(--font-size-sm);
  background: none;
  border: none;
  color: var(--accent);
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}
.guide:hover {
  color: var(--accent-hover);
}
.spacer {
  flex: 1;
}
.content {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-4);
}
.state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-8);
  color: var(--text-muted);
}
.state.error {
  color: var(--danger);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--space-4);
}
.tpl-card {
  padding: var(--space-3);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.preview {
  width: 100%;
  max-height: 220px;
  /* aspect-ratio is set inline from each template's effective dimensions. */
  background: #14171f;
  border-radius: var(--radius-md);
  overflow: hidden;
  border: 1px solid var(--border);
}
.preview img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.preview-ph {
  width: 100%;
  height: 100%;
}
.name {
  font-weight: 600;
  font-size: var(--font-size-sm);
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
  font-size: var(--font-size-xs);
  background: var(--bg-elevated);
  padding: 1px var(--space-2);
  border-radius: 999px;
  color: var(--text-secondary);
}
.desc {
  font-size: var(--font-size-xs);
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
