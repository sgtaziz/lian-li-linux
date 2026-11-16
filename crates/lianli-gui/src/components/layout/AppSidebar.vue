<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { RouterLink } from "vue-router";
import { invoke } from "@/utils/backendText";
import * as icons from "lucide-vue-next";
import { useTranslate } from "@/i18n";
import { MAIN_ROUTES } from "@/router";
import { useDaemonStore } from "@/stores/daemon";
import { useDevicesStore } from "@/stores/devices";
import { useConfigStore } from "@/stores/config";

const config = useConfigStore();
const daemon = useDaemonStore();
const devices = useDevicesStore();
const t = useTranslate();
const APP_VERSION = ref("...");

const UTILITY_ROUTES = new Set(["settings", "installation"]);
const primaryRoutes = computed(() => MAIN_ROUTES.filter((r) => !UTILITY_ROUTES.has(r.name)));
const utilityRoutes = computed(() => MAIN_ROUTES.filter((r) => UTILITY_ROUTES.has(r.name)));

onMounted(async () => {
  APP_VERSION.value = await invoke<string>("app_version");
});

function iconFor(name: string) {
  const map: Record<string, any> = {
    monitor: icons.Monitor,
    image: icons.Image,
    fan: icons.Fan,
    palette: icons.Palette,
    droplet: icons.Droplet,
    settings: icons.Settings,
    health: icons.HeartPulse,
  };
  return map[name] ?? icons.Circle;
}
</script>

<template>
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-mark"><icons.Fan :size="20" :stroke-width="2.2" /></div>
      <div class="brand-text">
        <div class="brand-title">{{ t("nav.appName") }}</div>
        <div class="brand-subtitle">v{{ APP_VERSION }}</div>
      </div>
    </div>

    <nav class="nav">
      <RouterLink
        v-for="r in primaryRoutes"
        :key="r.name"
        :to="r.to"
        class="nav-item"
        active-class="active"
        :title="t(r.labelKey)"
      >
        <component :is="iconFor(r.icon)" :size="20" :stroke-width="2" />
        <span class="nav-label">{{ t(r.labelKey) }}</span>
        <icons.TriangleAlert v-if="r.name === 'fans' && daemon.connected && devices.failingCurves.length" class="warn-icon" :size="16" :stroke-width="2.4" role="img" :aria-label="t('fans.sensorLost.navTitle')" />
        <span v-else-if="config.dirtyPages.includes(r.name)" class="dirty-dot" role="img" :aria-label="t('shell.unsaved.pageHasChanges')" :title="t('shell.unsaved.pageHasChanges')" />
      </RouterLink>
      <div class="nav-spacer" />
      <RouterLink
        v-for="r in utilityRoutes"
        :key="r.name"
        :to="r.to"
        class="nav-item"
        active-class="active"
        :title="t(r.labelKey)"
      >
        <component :is="iconFor(r.icon)" :size="20" :stroke-width="2" />
        <span class="nav-label">{{ t(r.labelKey) }}</span>
        <span v-if="config.dirtyPages.includes(r.name)" class="dirty-dot" role="img" :aria-label="t('shell.unsaved.pageHasChanges')" :title="t('shell.unsaved.pageHasChanges')" />
      </RouterLink>
    </nav>

    <div class="status" :class="{ ok: daemon.connected, bad: !daemon.connected }" :title="devices.summary">
      <div class="conn">
        <span class="dot" />
        <span class="status-text">{{ daemon.connected ? t("shell.sidebar.connected") : t("common.offline") }}</span>
      </div>
      <div class="device-count">{{ daemon.connected ? devices.summary : t("shell.sidebar.lastSeen", { summary: devices.summary }) }}</div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 256px;
  flex-shrink: 0;
  background: var(--md-surface-container);
  display: flex;
  flex-direction: column;
  padding: 0 var(--space-3) var(--space-4);
  box-sizing: border-box;
}
.brand {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  height: 72px;
  padding: 0 var(--space-2);
}
.brand-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  border-radius: 12px;
  background: var(--md-primary);
  color: var(--md-on-primary);
}
.brand-text {
  min-width: 0;
}
.brand-title {
  font-family: var(--font-display);
  font-weight: 600;
  font-size: 17px;
  line-height: 1.2;
  white-space: nowrap;
}
.brand-subtitle {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-top: var(--space-2);
  min-height: 0;
}
.nav-spacer {
  flex: 1;
  min-height: var(--space-4);
}
.nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  height: 48px;
  padding: 0 var(--space-4) 0 18px;
  border-radius: 24px;
  color: var(--md-on-surface-variant);
  font-size: var(--font-size-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.nav-item:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
  color: var(--md-on-surface);
}
.nav-item:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: -2px;
}
.nav-item.active {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
  font-weight: 600;
}
.nav-label {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.warn-icon {
  flex-shrink: 0;
  color: var(--warning);
}
.dirty-dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--warning);
}
.status {
  margin-top: var(--space-3);
  padding: 14px var(--space-4);
  border-radius: 20px;
  background: var(--md-surface-container-high);
}
.conn {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: 13px;
  font-weight: 600;
}
.dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
}
.status.ok .dot {
  background: var(--success);
  box-shadow: 0 0 8px var(--success);
}
.status.bad .dot {
  background: var(--danger);
}
.device-count {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
  margin-top: var(--space-1);
}
@media (max-width: 960px) {
  .sidebar {
    width: 88px;
    padding: 0 var(--space-3) var(--space-3);
  }
  .brand {
    justify-content: center;
    padding: 0;
  }
  .brand-text,
  .nav-label,
  .status-text,
  .device-count {
    display: none;
  }
  .nav-item {
    position: relative;
    justify-content: center;
    padding: 0;
  }
  .warn-icon,
  .dirty-dot {
    position: absolute;
    top: 10px;
    right: 14px;
  }
  .warn-icon {
    top: 2px;
    right: 4px;
  }
  .status {
    display: flex;
    justify-content: center;
    padding: var(--space-3);
  }
}
</style>
