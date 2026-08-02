<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import { darkTheme, type GlobalTheme, type GlobalThemeOverrides } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useThemeStore } from "@/stores/theme";
import { LCD_TEMPLATES_CHANGED_EVENT } from "@/stores/lcd";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";

const daemon = useDaemonStore();
const config = useConfigStore();
const theme = useThemeStore();
const route = useRoute();

// Secondary windows (editor/browser) render standalone, without the shell.
const isSecondaryWindow = computed(
  () => route.name === "editor" || route.name === "browser",
);

const darkOverrides: GlobalThemeOverrides = {
  common: {
    bodyColor: "#0f1117",
    cardColor: "#1a1d27",
    modalColor: "#1a1d27",
    popoverColor: "#232734",
    primaryColor: "#4f9eff",
    primaryColorHover: "#3a8be5",
    primaryColorPressed: "#2f7acc",
    textColorBase: "#e4e6eb",
    textColor1: "#e4e6eb",
    textColor2: "#c9cdd4",
    textColor3: "#9ba1ac",
    placeholderColor: "#6b7280",
    dividerColor: "#2e3340",
    borderColor: "#2e3340",
    inputColor: "#232734",
    inputColorDisabled: "#1a1d27",
    actionColor: "#232734",
    tableHeaderColor: "#1a1d27",
    hoverColor: "rgba(79, 158, 255, 0.12)",
    borderRadius: "8px",
    borderRadiusSmall: "6px",
  },
};

const lightOverrides: GlobalThemeOverrides = {
  common: {
    bodyColor: "#f4f6f9",
    cardColor: "#ffffff",
    modalColor: "#ffffff",
    popoverColor: "#ffffff",
    primaryColor: "#2f7acc",
    primaryColorHover: "#2569b0",
    primaryColorPressed: "#1d5499",
    textColorBase: "#1a1d27",
    textColor1: "#1a1d27",
    textColor2: "#3d4452",
    textColor3: "#565d6b",
    placeholderColor: "#868f9e",
    dividerColor: "#e2e6ee",
    borderColor: "#e2e6ee",
    inputColor: "#ffffff",
    inputColorDisabled: "#eef1f6",
    actionColor: "#eef1f6",
    tableHeaderColor: "#f4f6f9",
    hoverColor: "rgba(47, 122, 204, 0.12)",
    borderRadius: "8px",
    borderRadiusSmall: "6px",
  },
};

const naiveTheme = computed<GlobalTheme | null>(() =>
  theme.isDark ? darkTheme : null,
);
const themeOverrides = computed(() =>
  theme.isDark ? darkOverrides : lightOverrides,
);

let unlistenTemplatesChanged: UnlistenFn | undefined;

onMounted(async () => {
  // Kick off the initial config load + polling loop.
  await config.load().catch(() => undefined);
  daemon.start();

  // Each window (main/editor/browser) has its own store instance, so a
  // template saved in one (e.g. the editor) doesn't update the others on its
  // own — resync on this window's copy of the template list when notified.
  unlistenTemplatesChanged = await listen(LCD_TEMPLATES_CHANGED_EVENT, () => {
    config.load().catch(() => undefined);
  });
});

onUnmounted(() => {
  unlistenTemplatesChanged?.();
});
</script>

<template>
  <n-config-provider :theme="naiveTheme" :theme-overrides="themeOverrides">
    <n-message-provider>
      <n-dialog-provider>
        <n-notification-provider>
          <template v-if="isSecondaryWindow">
            <router-view v-slot="{ Component }">
              <transition name="page" mode="out-in">
                <component :is="Component" />
              </transition>
            </router-view>
          </template>
          <template v-else>
            <div class="shell">
              <AppSidebar />
              <div class="main">
                <AppHeader />
                <div class="content">
                  <router-view v-slot="{ Component }">
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
}
.content {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-6);
}
</style>
