<script setup lang="ts">
import { RouterLink } from "vue-router";
import * as icons from "lucide-vue-next";
import { MAIN_ROUTES } from "@/router";
import { useDaemonStore } from "@/stores/daemon";

const daemon = useDaemonStore();
const APP_VERSION = "0.6.1";

// Map icon name strings from the route table to Lucide components.
function iconFor(name: string) {
  const map: Record<string, any> = {
    monitor: icons.Monitor,
    image: icons.Image,
    fan: icons.Fan,
    palette: icons.Palette,
    droplet: icons.Droplet,
    settings: icons.Settings,
  };
  return map[name] ?? icons.Circle;
}
</script>

<template>
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-text">
        <div class="brand-title">Lian Li Linux</div>
        <div class="brand-subtitle">v{{ APP_VERSION }}</div>
      </div>
    </div>

    <nav class="nav">
      <RouterLink
        v-for="r in MAIN_ROUTES"
        :key="r.name"
        :to="r.to"
        class="nav-item"
        active-class="active"
      >
        <component :is="iconFor(r.icon)" :size="18" :stroke-width="2" />
        <span>{{ r.label }}</span>
      </RouterLink>
    </nav>

    <div class="footer">
      <div class="conn" :class="{ ok: daemon.connected, bad: !daemon.connected }">
        <span class="dot" />
        {{ daemon.connected ? "Connected" : "Offline" }}
      </div>
      <div class="device-count">{{ daemon.visibleDeviceCount }} device(s)</div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 200px;
  flex-shrink: 0;
  background: var(--bg-surface);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  padding: var(--space-4) var(--space-3);
}
.brand {
  padding: var(--space-2) var(--space-2) var(--space-4);
}
.brand-title {
  font-weight: 600;
  font-size: var(--font-size-base);
}
.brand-subtitle {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.nav {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  margin-top: var(--space-2);
}
.nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-size: var(--font-size-base);
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}
.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}
.nav-item.active {
  background: var(--accent-soft);
  color: var(--accent);
}
.footer {
  margin-top: auto;
  padding: var(--space-3) var(--space-2);
  border-top: 1px solid var(--border);
}
.conn {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
}
.conn .dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.conn.ok .dot {
  background: var(--success);
  box-shadow: 0 0 6px var(--success);
}
.conn.bad .dot {
  background: var(--danger);
}
.device-count {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  margin-top: var(--space-1);
}
</style>
