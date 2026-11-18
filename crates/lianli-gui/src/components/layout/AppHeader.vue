<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute } from "vue-router";
import { Moon, RefreshCw, Save, Sun } from "lucide-vue-next";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useThemeStore } from "@/stores/theme";
import { useTranslate, type MessageKey } from "@/i18n";
import { MAIN_ROUTES } from "@/router";
import { useSaveConfig } from "@/composables/useSaveConfig";

const daemon = useDaemonStore();
const config = useConfigStore();
const theme = useThemeStore();
const route = useRoute();
const t = useTranslate();
const { saving, save } = useSaveConfig();

type MainRouteName = (typeof MAIN_ROUTES)[number]["name"];

const SUBTITLE_KEYS: Record<MainRouteName, MessageKey> = {
  devices: "shell.pageSubtitle.devices",
  lcd: "shell.pageSubtitle.lcd",
  fans: "shell.pageSubtitle.fans",
  rgb: "shell.pageSubtitle.rgb",
  aio: "shell.pageSubtitle.aio",
  settings: "shell.pageSubtitle.settings",
  installation: "shell.pageSubtitle.installation",
};

const currentRoute = computed(() => MAIN_ROUTES.find((r) => r.name === route.name));
const title = computed(() => t(currentRoute.value?.labelKey ?? "nav.appName"));
const subtitle = computed(() => (currentRoute.value ? t(SUBTITLE_KEYS[currentRoute.value.name]) : ""));

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
  await save();
  (document.activeElement as HTMLElement | null)?.blur();
}

function onToggleTheme() {
  theme.toggle();
  (document.activeElement as HTMLElement | null)?.blur();
}
</script>

<template>
  <header class="header">
    <div class="heading">
      <h1 class="title">{{ title }}</h1>
      <p v-if="subtitle" class="subtitle">{{ subtitle }}</p>
    </div>
    <div class="spacer" />
    <n-button
      quaternary
      circle
      :aria-label="theme.isDark ? t('shell.header.switchToLight') : t('shell.header.switchToDark')"
      :title="theme.isDark ? t('shell.header.switchToLight') : t('shell.header.switchToDark')"
      @click="onToggleTheme"
    >
      <template #icon>
        <Sun v-if="theme.isDark" :size="18" />
        <Moon v-else :size="18" />
      </template>
    </n-button>
    <n-button
      quaternary
      circle
      :loading="refreshing"
      :disabled="refreshing"
      :aria-label="refreshing ? t('shell.header.refreshing') : t('shell.header.refreshNow')"
      :title="refreshing ? t('shell.header.refreshing') : t('shell.header.refreshNow')"
      @click="onRefresh"
    >
      <template #icon>
        <RefreshCw :size="18" :class="{ spin: refreshing }" />
      </template>
    </n-button>
    <n-button
      type="primary"
      :disabled="saving || !config.dirty || !daemon.canWrite"
      :loading="saving"
      :class="{ dirty: config.dirty }"
      @click="onSave"
    >
      <template #icon><Save :size="17" /></template>
      {{ t("common.save") }}{{ config.dirty ? "*" : "" }}
    </n-button>
  </header>
</template>

<style scoped>
.header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 28px 32px 20px 40px;
  flex-shrink: 0;
}
.heading {
  min-width: 0;
}
.title {
  font-family: var(--font-display);
  font-size: 34px;
  line-height: 1.15;
  font-weight: 600;
  letter-spacing: -0.3px;
  margin: 0;
}
.subtitle {
  margin: var(--space-1) 0 0;
  font-size: 15px;
  color: var(--md-on-surface-variant);
}
@media (max-width: 960px) {
  .header {
    padding: var(--space-6) var(--space-6) var(--space-4);
  }
  .title {
    font-size: 28px;
  }
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
  box-shadow: 0 0 0 2px var(--md-surface), 0 0 0 4px var(--warning);
}
</style>
