<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-shell";
import { AlertTriangle, CheckCircle2, Info, Loader2 } from "lucide-vue-next";
import { useIpc } from "@/composables/useIpc";
import { useDaemonStore } from "@/stores/daemon";
import type { AssetAccessReport, LcdConfig, LcdTemplate } from "@/types";

const props = defineProps<{ lcds: LcdConfig[]; templates: LcdTemplate[] }>();
const emit = defineEmits<{ issues: [present: boolean] }>();
const ipc = useIpc();
const daemon = useDaemonStore();
const report = ref<AssetAccessReport | null>(null);
const error = ref("");
const checking = ref(false);
const supported = computed(() => daemon.connected && daemon.info?.capabilities.includes("media_access"));
const hasSelection = computed(() => props.lcds.length > 0 || props.templates.length > 0);
const selection = computed(() => JSON.stringify({ lcds: props.lcds, templates: props.templates }));
const summary = computed(() => {
  if (checking.value) return "Checking file access…";
  if (error.value) return "File access could not be checked.";
  if (!daemon.connected) return "Connect to the daemon to check file access.";
  if (!supported.value) return "Update and restart the daemon to check file access.";
  if (report.value?.failed) return `${report.value.failed} file access ${report.value.failed === 1 ? "issue" : "issues"}`;
  if (report.value) return report.value.checked ? `${report.value.checked} media ${report.value.checked === 1 ? "dependency" : "dependencies"} readable` : "No external files to check";
  return "File access check scheduled…";
});
let timer: ReturnType<typeof setTimeout> | undefined;
let revision = 0;
let pending = false;
let disposed = false;

function schedule() {
  revision++;
  report.value = null;
  error.value = "";
  pending = false;
  clearTimeout(timer);
  if (!supported.value || !hasSelection.value) return;
  timer = setTimeout(() => {
    pending = true;
    void check();
  }, 600);
}

async function check() {
  if (checking.value || disposed || !pending) return;
  pending = false;
  const current = revision;
  checking.value = true;
  try {
    const result = await ipc.request<AssetAccessReport>("CheckMediaAccess", JSON.parse(selection.value));
    if (!disposed && current === revision) report.value = result;
  } catch (cause) {
    if (!disposed && current === revision) error.value = String(cause);
  } finally {
    checking.value = false;
    if (pending && !disposed) void check();
  }
}

watch([selection, supported, () => daemon.info?.instance_id, () => daemon.socketPath], schedule, { immediate: true });
watch(() => !!report.value?.failed, (present) => emit("issues", present), { immediate: true });
onUnmounted(() => {
  disposed = true;
  revision++;
  clearTimeout(timer);
});
</script>

<template>
  <div v-if="hasSelection" class="media-access" :class="{ 'has-issues': report?.failed || error }">
    <div class="access-row">
      <Loader2 v-if="checking" :size="16" class="status-icon checking" aria-hidden="true" />
      <AlertTriangle v-else-if="report?.failed || error" :size="16" class="status-icon warning" aria-hidden="true" />
      <CheckCircle2 v-else-if="report" :size="16" class="status-icon success" aria-hidden="true" />
      <Info v-else :size="16" class="status-icon" aria-hidden="true" />
      <span class="access-summary" role="status" aria-live="polite">{{ summary }}</span>
      <n-button v-if="supported" size="tiny" quaternary :disabled="checking" @click="schedule">Recheck</n-button>
      <n-button v-if="report?.failed" size="tiny" secondary @click="open('https://github.com/sgtaziz/lian-li-linux/blob/main/docs/lcd-assets.md')">File access guide</n-button>
      <slot name="actions" />
    </div>
    <details v-if="report?.failed || error" class="access-details">
      <summary>View details</summary>
      <p v-if="error">{{ error }}</p>
      <template v-if="report?.failed">
        <p>The {{ daemon.info?.mode }} daemon (UID {{ report.uid }}) needs access to these files and their parent folders.</p>
        <ul>
          <li v-for="(issue, index) in report.issues" :key="index">
            <strong>{{ issue.owner }}</strong><span v-if="issue.path"> — {{ issue.path }}</span>: {{ issue.error }}
          </li>
        </ul>
        <p v-if="report.failed > report.issues.length">Showing {{ report.issues.length }} of {{ report.failed }} issues.</p>
      </template>
    </details>
  </div>
</template>

<style scoped>
.media-access { min-width: 0; padding: 6px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); font-size: var(--font-size-sm); overflow-wrap: anywhere; }
.has-issues { border-left: 3px solid var(--warning); }
.access-row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); min-height: 22px; }
.access-summary { flex: 1; min-width: 160px; }
.status-icon { flex-shrink: 0; }
.warning { color: var(--warning); }
.success { color: var(--success); }
.access-details { margin: var(--space-1) 0 0 24px; }
.access-details summary { cursor: pointer; width: fit-content; color: var(--text-primary); }
.access-details p { margin: var(--space-2) 0 0; }
.access-details ul { margin: var(--space-2) 0 0; padding-left: var(--space-4); max-height: 10rem; overflow-y: auto; user-select: text; }
.access-details li + li { margin-top: var(--space-1); }
.checking { animation: spin 1.2s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .checking { animation: none; } }
</style>
