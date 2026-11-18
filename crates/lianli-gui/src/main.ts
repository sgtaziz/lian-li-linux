import { createApp } from "vue";
import { createPinia } from "pinia";
import naive from "naive-ui";
import TitledSelect from "./components/common/TitledSelect";
import App from "./App.vue";
import { router } from "./router";
import { i18n } from "./i18n";
import { migrateStoredKeys } from "./utils/storedPreference";
import "@fontsource-variable/outfit";
import "./styles/theme.css";

migrateStoredKeys();
const app = createApp(App);
app.use(createPinia());
app.use(router);
app.component("NSelect", TitledSelect);
app.use(naive);
app.use(i18n);
app.mount("#app");
