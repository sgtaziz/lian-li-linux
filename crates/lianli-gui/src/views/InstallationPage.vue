<script setup lang="ts">
import { computed, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-shell";
import { useMessage } from "naive-ui";
import { useDaemonStore } from "@/stores/daemon";
import { useInstallationStore } from "@/stores/installation";
import MediaStreamStatus from "@/components/common/MediaStreamStatus.vue";
import DiagnosticExport from "@/components/common/DiagnosticExport.vue";
import DesktopStreamStatus from "@/components/common/DesktopStreamStatus.vue";
import { INSTALLATION_GUIDES, type CheckState, type InstallationGuide } from "@/types/installation";

const installation = useInstallationStore();
const daemon = useDaemonStore();
const message = useMessage();
const checkedAt = computed(() => installation.report
  ? new Date(installation.report.checked_at_unix_ms).toLocaleString() : "Not checked yet");
const needsAttention = computed(() => (installation.report?.findings ?? []).filter((finding) => finding.state !== "passed" && finding.state !== "not_applicable"));
const otherFindings = computed(() => (installation.report?.findings ?? []).filter((finding) => finding.state === "passed" || finding.state === "not_applicable"));
const stateLabels: Record<CheckState, string> = {
  passed: "Passed", failed: "Failed", unavailable: "Unverified", not_applicable: "N/A",
};
onMounted(() => {
  if (!installation.report) void installation.recheck();
});

async function guide(id: InstallationGuide) {
  try {
    await open(INSTALLATION_GUIDES[id]);
  } catch (error) {
    message.error(`Could not open the guide: ${String(error)}`);
  }
}

async function recheck() {
  await Promise.all([installation.recheck(), daemon.refresh()]);
}
</script>

<template>
  <div class="health-page">
    <div class="health-header">
      <div><h1>Installation Health</h1><p>{{ installation.contextLabel }} · {{ checkedAt }}</p></div>
      <n-button :loading="installation.checking" @click="recheck">Recheck</n-button>
    </div>
    <n-alert v-if="installation.error" type="error" title="Check could not finish">
      {{ installation.error }} Previous results, if shown, may be out of date.
    </n-alert>
    <div class="connection-row">
      <n-tag :type="daemon.connected ? 'success' : 'warning'">{{ daemon.connected ? 'Connected' : 'Offline' }}</n-tag>
      <span v-if="daemon.info">{{ daemon.info.version }} · {{ daemon.info.mode }}</span>
      <router-link to="/settings">Service settings</router-link>
      <n-button size="small" text type="primary" @click="guide(installation.report?.context.kind === 'distrobox' ? 'distrobox' : 'service_modes')">Setup guide</n-button>
    </div>
    <p v-if="!daemon.connected" class="muted">Start a service in Settings, then Recheck.</p>
    <div v-if="needsAttention.length" class="check-list">
      <details v-for="finding in needsAttention" :key="finding.code" class="check-row">
        <summary>
          <n-tag size="small" :type="finding.severity === 'error' ? 'error' : 'warning'">{{ stateLabels[finding.state] }}</n-tag>
          <span class="check-text">
            <span class="check-title">{{ finding.title }}</span>
            <span class="check-sub">{{ finding.remediation || finding.evidence }}</span>
          </span>
        </summary>
        <div class="check-details">
          <p class="check-context">{{ finding.context }} · {{ finding.feature }}</p>
          <p>{{ finding.evidence }}</p>
          <n-button size="small" text type="primary" @click="guide(finding.guide)">Open guide</n-button>
        </div>
      </details>
    </div>
    <p v-else-if="installation.report" class="all-good">All checks that need attention pass.</p>
    <details v-if="otherFindings.length" class="passed-checks">
      <summary>{{ otherFindings.length }} passed or non-applicable checks</summary>
      <div class="check-list">
        <details v-for="finding in otherFindings" :key="finding.code" class="check-row">
          <summary>
            <n-tag size="small" :type="finding.state === 'passed' ? 'success' : 'default'">{{ stateLabels[finding.state] }}</n-tag>
            <span class="check-text">
              <span class="check-title">{{ finding.title }}</span>
              <span class="check-sub">{{ finding.feature }}</span>
            </span>
          </summary>
          <div class="check-details">
            <p>{{ finding.evidence }}</p>
            <n-button size="small" text type="primary" @click="guide(finding.guide)">Open guide</n-button>
          </div>
        </details>
      </div>
    </details>
    <MediaStreamStatus />
    <DesktopStreamStatus />
    <details class="diagnostics"><summary>Export diagnostics & daemon logs</summary><DiagnosticExport /></details>
    <details><summary>About these checks</summary><p>Checks cover installation, permissions, service state and the last configuration load. Recheck does not reload configuration or test physical playback.</p><p v-if="daemon.socketPath">{{ daemon.socketPath }}</p></details>
  </div>
</template>

<style scoped>
.health-page { display: flex; flex-direction: column; gap: var(--space-4); }
.health-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); }
h1 { font-size: var(--font-size-2xl); margin: 0; }
p { margin: var(--space-3) 0; overflow-wrap: anywhere; }
.check-context { color: var(--text-secondary); }
.check-list { border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--bg-surface); }
.check-row + .check-row { border-top: 1px solid var(--border); }
.check-row > summary { display: flex; align-items: flex-start; gap: var(--space-3); margin: 0; padding: var(--space-2) var(--space-3); color: var(--text-primary); list-style: none; }
.check-row > summary::-webkit-details-marker { display: none; }
.check-row > summary:hover { background: var(--bg-hover); }
.check-row > summary .n-tag { flex-shrink: 0; width: 82px; justify-content: center; }
.check-text { display: flex; flex-direction: column; min-width: 0; }
.check-title { font-weight: 600; font-size: var(--font-size-sm); }
.check-sub { font-size: var(--font-size-xs); color: var(--text-secondary); overflow-wrap: anywhere; }
.check-row:not([open]) .check-sub { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.check-details { padding: 0 var(--space-3) var(--space-2) calc(82px + var(--space-3) * 2); font-size: var(--font-size-sm); }
.check-details p { margin: var(--space-1) 0; }
.passed-checks > .check-list { margin-top: var(--space-2); }
.all-good { color: var(--success); margin: 0; }
.connection-row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); }
summary { cursor: pointer; color: var(--text-secondary); margin-bottom: var(--space-2); }
</style>
