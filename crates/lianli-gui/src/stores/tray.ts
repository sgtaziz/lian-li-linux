import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@/utils/backendText";
import { useDaemonStore } from "@/stores/daemon";
import { useDevicesStore } from "@/stores/devices";
import { useThemeStore } from "@/stores/theme";
import { tr } from "@/i18n";

export type TrayIconStyle = "auto" | "light" | "dark";

const CLOSE_TO_TRAY_KEY = "lianli-linux.closeToTray";
const ICON_STYLE_KEY = "lianli-linux.trayIcon";
const RETRY_MIN_MS = 2_000;
const RETRY_MAX_MS = 60_000;

function readStorage(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // The preference then lasts only for this session.
  }
}

export const useTrayStore = defineStore("tray", () => {
  const daemon = useDaemonStore();
  const devices = useDevicesStore();
  const theme = useThemeStore();

  const available = ref(false);
  const closeToTray = ref(readStorage(CLOSE_TO_TRAY_KEY) === "true");
  const storedStyle = readStorage(ICON_STYLE_KEY);
  const iconStyle = ref<TrayIconStyle>(storedStyle === "light" || storedStyle === "dark" ? storedStyle : "auto");
  const quitRequested = ref(0);
  const iconShown = ref(false);
  const hideOnClose = computed(() => available.value && closeToTray.value && iconShown.value);

  const glyph = computed(() => {
    if (iconStyle.value !== "auto") return iconStyle.value;
    return theme.desktopMode === "dark" ? "light" : "dark";
  });

  const status = computed(() => {
    const failing = daemon.connected ? devices.failingCurves : [];
    return {
      status: daemon.connected ? tr("shell.tray.connected", { summary: devices.summary }) : tr("shell.tray.offline"),
      warning: failing.length ? tr("shell.tray.sensorLost", { name: failing.join(", ") }) : null,
      tooltip: daemon.connected ? tr("shell.tray.tooltipConnected") : tr("shell.tray.tooltipOffline"),
      open: tr("shell.tray.open"),
      quit: tr("shell.tray.quit"),
      glyph: glyph.value,
      state: !daemon.connected ? "offline" : failing.length ? "warning" : "normal",
      visible: closeToTray.value,
    };
  });

  function setCloseToTray(value: boolean) {
    closeToTray.value = value;
    writeStorage(CLOSE_TO_TRAY_KEY, String(value));
  }

  function setIconStyle(value: TrayIconStyle) {
    iconStyle.value = value;
    writeStorage(ICON_STYLE_KEY, value);
  }

  function setHidden(hidden: boolean) {
    daemon.setBackground(hidden);
  }

  let sent = "";
  let retry: ReturnType<typeof setTimeout> | undefined;
  let retryDelay = RETRY_MIN_MS;

  function push(next: typeof status.value) {
    clearTimeout(retry);
    retry = undefined;
    const key = JSON.stringify(next);
    if (key === sent) return;
    sent = key;
    invoke("set_tray_status", { status: next })
      .then(() => {
        if (sent !== key) return;
        retryDelay = RETRY_MIN_MS;
        iconShown.value = next.visible;
      })
      .catch(() => {
        if (sent !== key) return;
        sent = "";
        iconShown.value = false;
        retry = setTimeout(() => push(status.value), retryDelay);
        retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
      });
  }

  let started = false;
  async function start() {
    if (started) return;
    started = true;
    const trayRunning = await invoke<boolean>("tray_available").catch(() => false);
    if (!trayRunning) return;
    const unlisten: UnlistenFn[] = [];
    try {
      unlisten.push(await listen("tray://shown", () => setHidden(false)));
      unlisten.push(
        await listen("tray://quit", () => {
          quitRequested.value += 1;
        }),
      );
    } catch {
      unlisten.forEach((stop) => stop());
      started = false;
      return;
    }
    available.value = true;
    watch(status, push, { immediate: true });
  }

  async function quit() {
    await invoke("quit_app");
  }

  return {
    available,
    closeToTray,
    iconStyle,
    hideOnClose,
    quitRequested,
    setCloseToTray,
    setIconStyle,
    setHidden,
    start,
    quit,
  };
});
