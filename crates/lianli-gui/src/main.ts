import { createApp } from "vue";
import { createPinia } from "pinia";
import { router } from "./router";
import App from "./App.vue";
import "./style.css";

const isDark = localStorage.getItem("theme") !== "light";
document.documentElement.classList.toggle("dark", isDark);

const app = createApp(App);
app.use(createPinia());
app.use(router);
app.mount("#app");
