<script setup lang="ts">
import { computed, type Component } from "vue";
import { useRouter } from "vue-router";
import { Droplet, Fan, MonitorOff, Palette, PlugZap, Power } from "lucide-vue-next";
import { useDaemonStore } from "@/stores/daemon";
import { useExternalLink } from "@/composables/useExternalLink";
import { INSTALLATION_GUIDES } from "@/types/installation";
import { useTranslate, type MessageKey } from "@/i18n";

type PageKind = "devices" | "lcd" | "fans" | "rgb" | "aio";

const props = defineProps<{ kind: PageKind; compact?: boolean }>();

const daemon = useDaemonStore();
const router = useRouter();
const openExternal = useExternalLink();
const t = useTranslate();

interface Action {
  label: MessageKey;
  run: () => unknown;
}

interface Content {
  icon: Component;
  tone: "warning" | "primary" | "secondary";
  title: MessageKey;
  body: MessageKey;
  primary: Action;
  secondary?: Action;
}

const goDevices: Action = { label: "shell.emptyState.goDevices", run: () => router.push("/devices") };

const PAGE_CONTENT: Record<PageKind, Content> = {
  devices: {
    icon: PlugZap,
    tone: "primary",
    title: "shell.emptyState.devicesTitle",
    body: "shell.emptyState.devicesBody",
    primary: { label: "shell.emptyState.runHealth", run: () => router.push("/installation") },
    secondary: { label: "common.retry", run: () => daemon.refresh() },
  },
  lcd: { icon: MonitorOff, tone: "secondary", title: "shell.emptyState.lcdTitle", body: "shell.emptyState.lcdBody", primary: goDevices },
  fans: { icon: Fan, tone: "secondary", title: "shell.emptyState.fansTitle", body: "shell.emptyState.fansBody", primary: goDevices },
  rgb: { icon: Palette, tone: "secondary", title: "shell.emptyState.rgbTitle", body: "shell.emptyState.rgbBody", primary: goDevices },
  aio: { icon: Droplet, tone: "secondary", title: "shell.emptyState.aioTitle", body: "shell.emptyState.aioBody", primary: goDevices },
};

const DAEMON_OFFLINE: Content = {
  icon: Power,
  tone: "warning",
  title: "shell.emptyState.daemonTitle",
  body: "shell.emptyState.daemonBody",
  primary: { label: "shell.emptyState.chooseService", run: () => router.push("/settings") },
  secondary: { label: "shell.emptyState.troubleshooting", run: () => openExternal(INSTALLATION_GUIDES.troubleshooting) },
};

const content = computed(() => (daemon.connected ? PAGE_CONTENT[props.kind] : DAEMON_OFFLINE));
</script>

<template>
  <div class="empty-state" :class="{ compact }">
    <span class="empty-icon" :class="`tone-${content.tone}`">
      <component :is="content.icon" :size="compact ? 22 : 28" :stroke-width="2" />
    </span>
    <h2 class="empty-title">{{ t(content.title) }}</h2>
    <p class="empty-body">{{ t(content.body) }}</p>
    <div class="empty-actions">
      <n-button type="primary" round @click="content.primary.run()">{{ t(content.primary.label) }}</n-button>
      <n-button v-if="content.secondary" secondary round @click="content.secondary.run()">{{ t(content.secondary.label) }}</n-button>
    </div>
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: 48px var(--space-6);
  border-radius: 24px;
  background: var(--md-surface-container-low);
  text-align: center;
}
.empty-state.compact {
  padding: var(--space-6) var(--space-4);
  background: var(--md-surface-container-high);
}
.empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 22px;
}
.compact .empty-icon {
  width: 48px;
  height: 48px;
  border-radius: 16px;
}
.tone-warning {
  background: color-mix(in srgb, var(--warning) 18%, var(--md-surface-container-high));
  color: var(--warning);
}
.tone-primary {
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.tone-secondary {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.empty-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 22px;
  font-weight: 600;
  color: var(--md-on-surface);
}
.compact .empty-title {
  font-size: 18px;
}
.empty-body {
  max-width: 460px;
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: var(--md-on-surface-variant);
}
.empty-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
</style>
