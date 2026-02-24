import { createRouter, createWebHistory } from "vue-router";
import DevicesView from "./views/DevicesView.vue";
import LcdView from "./views/LcdView.vue";
import FansView from "./views/FansView.vue";
import RgbView from "./views/RgbView.vue";
import SettingsView from "./views/SettingsView.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", name: "devices", component: DevicesView },
    { path: "/lcd", name: "lcd", component: LcdView },
    { path: "/fans", name: "fans", component: FansView },
    { path: "/rgb", name: "rgb", component: RgbView },
    { path: "/settings", name: "settings", component: SettingsView },
  ],
});
