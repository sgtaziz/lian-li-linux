<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute } from "vue-router";
import { RefreshCw, Save } from "lucide-vue-next";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { MAIN_ROUTES } from "@/router";

const daemon = useDaemonStore();
const config = useConfigStore();
const route = useRoute();

const title = computed(
  () => MAIN_ROUTES.find((r) => r.name === route.name)?.label ?? "Lian Li Linux",
);

const refreshing = ref(false);

async function onRefresh() {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    await daemon.refresh();
  } finally {
    refreshing.value = false;
    // Drop focus so the button doesn't stay in its active/pressed colour.
    (document.activeElement as HTMLElement | null)?.blur();
  }
}

async function onSave() {
  await config.save();
  (document.activeElement as HTMLElement | null)?.blur();
}
</script>

<template>
  <header class="header">
    <h1 class="title">{{ title }}</h1>
    <div class="spacer" />
    <n-button
      quaternary
      size="small"
      :loading="refreshing"
      :disabled="refreshing"
      :title="refreshing ? 'Refreshing…' : 'Refresh now'"
      @click="onRefresh"
    >
      <template #icon>
        <RefreshCw :size="15" :class="{ spin: refreshing }" />
      </template>
    </n-button>
    <n-button
      type="primary"
      size="small"
      :disabled="!config.dirty"
      :class="{ dirty: config.dirty }"
      @click="onSave"
    >
      <template #icon><Save :size="15" /></template>
      Save{{ config.dirty ? "*" : "" }}
    </n-button>
  </header>
</template>

<style scoped>
.header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-6);
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
  flex-shrink: 0;
}
.title {
  font-size: var(--font-size-xl);
  font-weight: 600;
  margin: 0;
}
.spacer {
  flex: 1;
}
.spin {
  animation: header-spin 0.9s linear infinite;
}
@keyframes header-spin {
  to {
    transform: rotate(360deg);
  }
}
.dirty {
  box-shadow: 0 0 0 1px var(--warning), 0 0 8px rgba(251, 191, 36, 0.4);
}
</style>

