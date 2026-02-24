<script setup lang="ts">
import { ref } from "vue";
import { useDeviceStore } from "../stores/devices";
import { useConfigStore } from "../stores/config";
import { Monitor, Image, Fan, Palette, Settings, Sun, Moon } from "lucide-vue-next";

const deviceStore = useDeviceStore();
const configStore = useConfigStore();

const isDark = ref(document.documentElement.classList.contains("dark"));

function toggleTheme() {
  isDark.value = !isDark.value;
  document.documentElement.classList.toggle("dark", isDark.value);
  localStorage.setItem("theme", isDark.value ? "dark" : "light");
}

const navItems = [
  { path: "/", label: "Devices", icon: Monitor },
  { path: "/lcd", label: "LCD", icon: Image },
  { path: "/fans", label: "Fans", icon: Fan },
  { path: "/rgb", label: "RGB", icon: Palette },
  { path: "/settings", label: "Settings", icon: Settings },
];
</script>

<template>
  <aside
    class="w-56 shrink-0 bg-gray-50 dark:bg-gray-800 border-r border-gray-200 dark:border-gray-700 flex flex-col"
  >
    <div class="px-4 py-4 border-b border-gray-200 dark:border-gray-700">
      <h1 class="text-lg font-bold">Lian Li Linux</h1>
      <div class="mt-1 flex items-center gap-1.5 text-xs">
        <span
          class="w-2 h-2 rounded-full"
          :class="deviceStore.daemonConnected ? 'bg-green-500' : 'bg-red-500'"
        />
        <span class="text-gray-500 dark:text-gray-400">
          {{ deviceStore.daemonConnected ? "Daemon connected" : "Daemon offline" }}
        </span>
      </div>
    </div>

    <nav class="flex-1 px-2 py-3 space-y-0.5">
      <router-link
        v-for="item in navItems"
        :key="item.path"
        :to="item.path"
        class="flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-colors"
        :class="
          $route.path === item.path
            ? 'bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 font-medium'
            : 'text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700/50'
        "
      >
        <component :is="item.icon" :size="18" :stroke-width="1.75" />
        <span>{{ item.label }}</span>
      </router-link>
    </nav>

    <div class="px-4 py-3 border-t border-gray-200 dark:border-gray-700 text-xs text-gray-400">
      <div class="flex items-center justify-between">
        <div>
          <div>{{ deviceStore.devices.length }} device(s)</div>
          <div v-if="configStore.dirty" class="text-amber-500 mt-0.5">Unsaved changes</div>
        </div>
        <button
          @click="toggleTheme"
          class="p-1.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
          :title="isDark ? 'Switch to light mode' : 'Switch to dark mode'"
        >
          <Moon v-if="isDark" :size="14" />
          <Sun v-else :size="14" />
        </button>
      </div>
    </div>
  </aside>
</template>
