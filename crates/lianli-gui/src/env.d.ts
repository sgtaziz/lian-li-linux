/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}

interface Window {
  __TAURI_INTERNALS__?: any;
  __TAURI__?: any;
  __LIANLI_WINDOW__?: string;
}
