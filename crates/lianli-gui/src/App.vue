<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import { darkTheme, dateZhCN, zhCN, type GlobalTheme } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useDaemonStore } from "@/stores/daemon";
import { useTrayStore } from "@/stores/tray";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useConfigStore } from "@/stores/config";
import { useThemeStore } from "@/stores/theme";
import { useLanguageStore } from "@/stores/language";
import { LCD_TEMPLATES_CHANGED_EVENT } from "@/stores/lcd";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";
import InstallationNotice from "@/components/common/InstallationNotice.vue";
import CompatibilityNotice from "@/components/common/CompatibilityNotice.vue";
import WirelessNotice from "@/components/common/WirelessNotice.vue";
import UpdateNotice from "@/components/common/UpdateNotice.vue";
import UnsavedChanges from "@/components/layout/UnsavedChanges.vue";
import PageLoading from "@/components/layout/PageLoading.vue";

const daemon = useDaemonStore();
const tray = useTrayStore();
const config = useConfigStore();
const theme = useThemeStore();
const language = useLanguageStore();
const route = useRoute();
const naiveLocale = computed(() => (language.locale === "zh-CN" ? zhCN : null));
const naiveDateLocale = computed(() => (language.locale === "zh-CN" ? dateZhCN : null));

// Secondary windows (editor/browser) render standalone, without the shell.
const isSecondaryWindow = computed(
  () => route.name === "editor" || route.name === "browser",
);

const naiveTheme = computed<GlobalTheme | null>(() =>
  theme.isDark ? darkTheme : null,
);
const themeOverrides = computed(() => theme.overrides);

let unlistenTemplatesChanged: UnlistenFn | undefined;

function isMainWindow(): boolean {
  try {
    return getCurrentWindow().label === "main";
  } catch {
    return false;
  }
}

onMounted(async () => {
  theme.start();
  // Kick off the initial config load + polling loop.
  await config.load().catch(() => undefined);
  daemon.start();
  if (isMainWindow()) void tray.start();

  unlistenTemplatesChanged = await listen(LCD_TEMPLATES_CHANGED_EVENT, () => {
    config.refreshTemplates().catch(() => undefined);
  });
});

onUnmounted(() => {
  unlistenTemplatesChanged?.();
});
</script>

<template>
  <n-config-provider
    :theme="naiveTheme"
    :theme-overrides="themeOverrides"
    :locale="naiveLocale"
    :date-locale="naiveDateLocale"
  >
    <n-message-provider>
      <n-dialog-provider>
        <n-notification-provider>
          <template v-if="isSecondaryWindow">
            <CompatibilityNotice />
            <router-view v-slot="{ Component }">
              <transition name="page" mode="out-in">
                <component :is="Component" />
              </transition>
            </router-view>
          </template>
          <template v-else>
            <InstallationNotice />
            <div class="shell">
              <AppSidebar />
              <div class="main">
                <AppHeader />
                <div class="content">
                  <div class="notices">
                    <UnsavedChanges />
                    <CompatibilityNotice />
                    <WirelessNotice />
                    <UpdateNotice />
                  </div>
                  <PageLoading v-if="!daemon.ready" />
                  <router-view v-else v-slot="{ Component }">
                    <transition name="page" mode="out-in">
                      <component :is="Component" />
                    </transition>
                  </router-view>
                </div>
              </div>
            </div>
          </template>
        </n-notification-provider>
      </n-dialog-provider>
    </n-message-provider>
  </n-config-provider>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100vh;
  width: 100vw;
}
.main {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  background: var(--md-surface);
}
.content {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-2) 40px 40px;
}
@media (max-width: 960px) {
  .content {
    padding: var(--space-2) var(--space-6) var(--space-6);
  }
}
.notices {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.notices:not(:empty) {
  margin-bottom: var(--space-4);
}
</style>
