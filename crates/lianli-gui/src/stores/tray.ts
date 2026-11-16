import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@/utils/backendText";
import { useDaemonStore } from "@/stores/daemon";
import { useDevicesStore } from "@/stores/devices";
import { useThemeStore } from "@/stores/theme";
import { tr } from "@/i18n";

export type TrayIconStyle = "auto" | "light" | "dark";

const CLOSE_TO_TRAY_KEY = "lianli-linux.closeToTray";
const ICON_STYLE_KEY = "lianli-linux.trayIcon";

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
  const hideOnClose = computed(() => available.value && closeToTray.value);

  const glyph = computed(() => {
    if (iconStyle.value !== "auto") return iconStyle.value;
    const desktopDark = (theme.desktopScheme ?? theme.mode) === "dark";
    return desktopDark ? "light" : "dark";
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

  let started = false;
  async function start() {
    if (started) return;
    started = true;
    try {
      available.value = await invoke<boolean>("tray_available");
    } catch {
      available.value = false;
    }
    if (!available.value) return;
    let sent = "";
    watch(
      status,
      (next) => {
        const key = JSON.stringify(next);
        if (key === sent) return;
        sent = key;
        void invoke("set_tray_status", { status: next }).catch(() => {
          sent = "";
        });
      },
      { immediate: true },
    );
    await listen("tray://shown", () => setHidden(false));
    await listen("tray://quit", () => {
      quitRequested.value += 1;
    });
  }

  async function quit() {
    await invoke("quit_app");
  }

  return { available, closeToTray, iconStyle, hideOnClose, quitRequested, setCloseToTray, setIconStyle, setHidden, start, quit };
});
