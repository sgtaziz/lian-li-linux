<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import { darkTheme, type GlobalThemeOverrides } from "naive-ui";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";

const daemon = useDaemonStore();
const config = useConfigStore();
const route = useRoute();

// Secondary windows (editor/browser) render standalone, without the shell.
const isSecondaryWindow = computed(
  () => route.name === "editor" || route.name === "browser",
);

const themeOverrides: GlobalThemeOverrides = {
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

onMounted(async () => {
  // Kick off the initial config load + polling loop.
  await config.load().catch(() => undefined);
  daemon.start();
});
</script>

<template>
  <n-config-provider :theme="darkTheme" :theme-overrides="themeOverrides">
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
