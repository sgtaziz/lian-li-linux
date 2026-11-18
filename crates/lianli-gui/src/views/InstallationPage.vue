<script setup lang="ts">
import { computed, onMounted } from "vue";
import { AlertTriangle, Check, ChevronDown, CircleHelp, Minus, X } from "lucide-vue-next";
import { useExternalLink } from "@/composables/useExternalLink";
import { useDaemonStore } from "@/stores/daemon";
import { useInstallationStore } from "@/stores/installation";
import MediaStreamStatus from "@/components/common/MediaStreamStatus.vue";
import DiagnosticExport from "@/components/common/DiagnosticExport.vue";
import DesktopStreamStatus from "@/components/common/DesktopStreamStatus.vue";
import { INSTALLATION_GUIDES, type CheckState, type InstallationFinding, type InstallationGuide } from "@/types/installation";
import { DAEMON_MODE_LABELS } from "@/utils/serviceSetup";
import { findingContext, findingFeature, findingRemediation, findingTitle } from "@/utils/findingText";
import { useTranslate, type MessageKey } from "@/i18n";

const t = useTranslate();
const installation = useInstallationStore();
const daemon = useDaemonStore();
const checkedAt = computed(() => installation.report
  ? new Date(installation.report.checked_at_unix_ms).toLocaleString() : t("service.installation.notChecked"));
const needsAttention = computed(() => (installation.report?.findings ?? []).filter((finding) => finding.state !== "passed" && finding.state !== "not_applicable"));
const otherFindings = computed(() => (installation.report?.findings ?? []).filter((finding) => finding.state === "passed" || finding.state === "not_applicable"));
const passedCount = computed(() => otherFindings.value.filter((finding) => finding.state === "passed").length);
const stateLabels: Record<CheckState, MessageKey> = {
  passed: "service.installation.state.passed",
  failed: "service.installation.state.failed",
  unavailable: "service.installation.state.unavailable",
  not_applicable: "service.installation.state.notApplicable",
};

const heroTone = computed(() => {
  if (installation.error) return "failed";
  if (!installation.report) return "unknown";
  if (needsAttention.value.some((finding) => finding.severity === "error")) return "error";
  if (needsAttention.value.length) return "warning";
  return "good";
});
const heroHeadline = computed<MessageKey>(() => {
  if (heroTone.value === "failed") return "service.installation.checkFailedTitle";
  if (heroTone.value === "good") return "service.installation.everythingGood";
  return installation.report ? "service.installation.needsAttention" : "service.installation.notChecked";
});

function tone(finding: InstallationFinding): string {
  if (finding.state === "passed") return "passed";
  if (finding.state === "not_applicable") return "muted";
  return finding.severity === "error" ? "error" : "warning";
}

function toneIcon(finding: InstallationFinding) {
  if (finding.state === "passed") return Check;
  if (finding.state === "not_applicable") return Minus;
  if (finding.state === "unavailable") return CircleHelp;
  return finding.severity === "error" ? X : AlertTriangle;
}

onMounted(() => {
  if (!installation.report) void installation.recheck();
});

const openExternal = useExternalLink();
async function guide(id: InstallationGuide) {
  await openExternal(INSTALLATION_GUIDES[id]);
}

async function recheck() {
  await Promise.all([installation.recheck(), daemon.refresh()]);
}
</script>

<template>
  <div class="health-page">
    <n-alert v-if="installation.error" type="error" :title="t('service.installation.checkFailedTitle')">
      {{ t("service.installation.checkFailed", { error: installation.error }) }}
    </n-alert>

    <section class="status" :class="`status-${heroTone}`">
      <div class="status-main">
        <div class="headline">
          <span v-if="heroTone === 'good'" class="good-mark"><Check :size="16" :stroke-width="2.8" /></span>
          <span v-else-if="heroTone === 'failed'" class="count"><X :size="20" :stroke-width="2.8" /></span>
          <span v-else-if="installation.report" class="count">{{ needsAttention.length }}</span>
          <span class="headline-text">{{ t(heroHeadline) }}</span>
        </div>
        <div class="chips">
          <span class="chip">{{ installation.contextLabel }}</span>
          <span class="chip daemon-chip" :class="{ offline: !daemon.connected }">
            <span class="dot" />{{ t("service.installation.daemonTile") }}:
            {{ t(daemon.connected ? "service.installation.connected" : "service.installation.offline") }}
            <template v-if="daemon.info"> · {{ daemon.info.version }} · {{ t(DAEMON_MODE_LABELS[daemon.info.mode]) }}</template>
          </span>
          <span v-if="installation.report" class="chip"><Check :size="14" />{{ passedCount }} {{ t("service.installation.passedTile") }}</span>
          <span class="chip muted">{{ t("service.installation.lastChecked", { time: checkedAt }) }}</span>
        </div>
      </div>
      <div class="status-actions">
        <n-button size="small" type="primary" :loading="installation.checking" @click="recheck">{{ t("service.installation.recheck") }}</n-button>
        <n-button size="small" secondary @click="guide(installation.report?.context.kind === 'distrobox' ? 'distrobox' : 'service_modes')">
          {{ t("service.installation.setupGuide") }}
        </n-button>
        <router-link class="settings-link" to="/settings">{{ t("service.installation.serviceSettings") }}</router-link>
      </div>
    </section>

    <p v-if="!daemon.connected" class="hint">{{ t("service.installation.startServiceHint") }}</p>

    <section v-if="needsAttention.length" class="card list-card">
      <h2 class="section-title">{{ t("service.installation.needsAttention") }}</h2>
      <div class="check-list">
        <details v-for="finding in needsAttention" :key="finding.code" class="check-row">
          <summary>
            <span class="state-icon" :class="`tone-${tone(finding)}`"><component :is="toneIcon(finding)" :size="18" :stroke-width="2.4" /></span>
            <span class="check-text">
              <span class="check-title">{{ findingTitle(finding) }}</span>
              <span class="check-sub">{{ findingRemediation(finding) || finding.evidence }}</span>
            </span>
            <span class="state-chip" :class="`tone-${tone(finding)}`">{{ t(stateLabels[finding.state]) }}</span>
            <ChevronDown class="chevron" :size="18" />
          </summary>
          <div class="check-details">
            <p class="check-context">{{ findingContext(finding) }} · {{ findingFeature(finding) }}</p>
            <p>{{ finding.evidence }}</p>
            <n-button size="small" secondary @click="guide(finding.guide)">{{ t("service.installation.openGuide") }}</n-button>
          </div>
        </details>
      </div>
    </section>

    <details v-if="otherFindings.length" class="card fold">
      <summary class="fold-summary">
        <span>{{ t("service.installation.otherChecks", { count: otherFindings.length }) }}</span>
        <ChevronDown class="chevron" :size="18" />
      </summary>
      <div class="check-list">
        <details v-for="finding in otherFindings" :key="finding.code" class="check-row">
          <summary>
            <span class="state-icon" :class="`tone-${tone(finding)}`"><component :is="toneIcon(finding)" :size="18" :stroke-width="2.4" /></span>
            <span class="check-text">
              <span class="check-title">{{ findingTitle(finding) }}</span>
              <span class="check-sub">{{ findingFeature(finding) }}</span>
            </span>
            <span class="state-chip" :class="`tone-${tone(finding)}`">{{ t(stateLabels[finding.state]) }}</span>
            <ChevronDown class="chevron" :size="18" />
          </summary>
          <div class="check-details">
            <p>{{ finding.evidence }}</p>
            <n-button size="small" secondary @click="guide(finding.guide)">{{ t("service.installation.openGuide") }}</n-button>
          </div>
        </details>
      </div>
    </details>

    <MediaStreamStatus />
    <DesktopStreamStatus />

    <details class="card fold diagnostics">
      <summary class="fold-summary">
        <span>{{ t("service.installation.exportDiagnostics") }}</span>
        <ChevronDown class="chevron" :size="18" />
      </summary>
      <DiagnosticExport />
    </details>
    <details class="card fold">
      <summary class="fold-summary">
        <span>{{ t("service.installation.aboutChecks") }}</span>
        <ChevronDown class="chevron" :size="18" />
      </summary>
      <p>{{ t("service.installation.aboutChecksText") }}</p>
      <p v-if="daemon.socketPath" class="mono">{{ daemon.socketPath }}</p>
    </details>
  </div>
</template>

<style scoped>
.health-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
p {
  margin: var(--space-2) 0;
  overflow-wrap: anywhere;
}
.status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3) var(--space-6);
  padding: var(--space-3) 20px;
  border-radius: 20px;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
}
.status-good {
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.status-warning {
  background: color-mix(in srgb, var(--warning) 22%, var(--md-surface-container));
}
.status-error,
.status-failed {
  background: color-mix(in srgb, var(--danger) 22%, var(--md-surface-container));
}
.status-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.headline {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.count {
  font-family: var(--font-display);
  font-size: 24px;
  font-weight: 600;
  line-height: 1;
}
.status-warning .count {
  color: var(--warning);
}
.status-error .count,
.status-failed .count {
  color: var(--danger);
}
.headline-text {
  font-family: var(--font-display);
  font-size: 18px;
  font-weight: 600;
}
.good-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: var(--md-primary);
  color: var(--md-on-primary);
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  border-radius: 10px;
  background: color-mix(in srgb, currentColor 12%, transparent);
  font-size: var(--font-size-xs);
  font-weight: 600;
}
.chip.muted {
  font-weight: 500;
  opacity: 0.8;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--success);
  box-shadow: 0 0 8px var(--success);
}
.daemon-chip.offline .dot {
  background: var(--danger);
  box-shadow: none;
}
.status-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.settings-link {
  color: inherit;
  font-size: var(--font-size-sm);
  font-weight: 600;
  text-decoration: underline;
  text-underline-offset: 2px;
}
.list-card .section-title {
  margin-bottom: var(--space-3);
}
.check-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.check-row {
  border-radius: 18px;
  background: var(--md-surface-container-high);
}
.check-row > summary {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4) var(--space-3) var(--space-3);
  border-radius: 18px;
  list-style: none;
  cursor: pointer;
}
.check-row > summary::-webkit-details-marker,
.fold-summary::-webkit-details-marker {
  display: none;
}
.check-row > summary:hover {
  background: color-mix(in srgb, var(--md-on-surface) 6%, transparent);
}
.check-row > summary:focus-visible,
.fold-summary:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -2px;
}
.state-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  border-radius: 14px;
}
.state-chip {
  flex-shrink: 0;
  padding: 3px 10px;
  border-radius: 10px;
  font-size: var(--font-size-xs);
  font-weight: 700;
}
.tone-error {
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
}
.tone-warning {
  background: color-mix(in srgb, var(--warning) 18%, transparent);
  color: var(--warning);
}
.tone-passed {
  background: color-mix(in srgb, var(--success) 18%, transparent);
  color: var(--success);
}
.tone-muted {
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
}
.check-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.check-title {
  font-size: 14px;
  font-weight: 600;
}
.check-sub {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
  overflow-wrap: anywhere;
}
.check-row:not([open]) .check-sub {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.chevron {
  flex-shrink: 0;
  color: var(--md-on-surface-variant);
  transition: transform 0.2s;
}
details[open] > summary > .chevron {
  transform: rotate(180deg);
}
.check-details {
  padding: 0 var(--space-4) var(--space-4) calc(40px + var(--space-3) * 2);
  font-size: var(--font-size-sm);
}
.check-details p {
  margin: var(--space-1) 0 var(--space-2);
}
.check-context {
  color: var(--md-on-surface-variant);
}
.fold {
  padding: var(--space-4) var(--space-6);
}
.fold-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  min-height: 32px;
  list-style: none;
  cursor: pointer;
  font-family: var(--font-display);
  font-size: var(--font-size-lg);
  font-weight: 600;
}
.fold[open] > .fold-summary {
  margin-bottom: var(--space-3);
}
.mono {
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
</style>
