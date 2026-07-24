import { createRouter, createWebHashHistory, type RouteRecordRaw } from "vue-router";

// Six primary pages + the two secondary-window routes (editor/browser) that
// share the same frontend bundle. The sidebar only shows the six main pages.
const routes: RouteRecordRaw[] = [
  { path: "/", redirect: "/devices" },
  { path: "/devices", name: "devices", component: () => import("@/views/DevicesPage.vue") },
  { path: "/lcd", name: "lcd", component: () => import("@/views/LcdPage.vue") },
  { path: "/fans", name: "fans", component: () => import("@/views/FansPage.vue") },
  { path: "/rgb", name: "rgb", component: () => import("@/views/RgbPage.vue") },
  { path: "/aio", name: "aio", component: () => import("@/views/AioPage.vue") },
  { path: "/settings", name: "settings", component: () => import("@/views/SettingsPage.vue") },
  // Secondary windows (rendered without the app shell).
  { path: "/editor", name: "editor", component: () => import("@/views/EditorWindow.vue") },
  { path: "/browser", name: "browser", component: () => import("@/views/BrowserWindow.vue") },
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

export const MAIN_ROUTES = [
  { name: "devices", label: "Devices", icon: "monitor", to: "/devices" },
  { name: "lcd", label: "LCD", icon: "image", to: "/lcd" },
  { name: "fans", label: "Fans", icon: "fan", to: "/fans" },
  { name: "rgb", label: "RGB", icon: "palette", to: "/rgb" },
  { name: "aio", label: "AIO", icon: "droplet", to: "/aio" },
  { name: "settings", label: "Settings", icon: "settings", to: "/settings" },
] as const;
