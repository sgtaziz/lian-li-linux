<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { AlertTriangle } from "lucide-vue-next";
import { useConfigStore } from "@/stores/config";
import { useDaemonStore } from "@/stores/daemon";
import { useTrayStore } from "@/stores/tray";
import { useSaveConfig } from "@/composables/useSaveConfig";
import { useTranslate } from "@/i18n";

const config = useConfigStore();
const daemon = useDaemonStore();
const tray = useTrayStore();
const t = useTranslate();
const { saving, save } = useSaveConfig();

const closeAsked = ref(false);
let closeConfirmed = false;
let quitAfterClose = false;
let unlistenClose: UnlistenFn | undefined;

function onKeydown(event: KeyboardEvent) {
  if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "s") {
    event.preventDefault();
    if (!event.repeat) void save();
  }
}

async function closeWindow() {
  closeConfirmed = true;
  closeAsked.value = false;
  try {
    if (quitAfterClose) await tray.quit();
    else await getCurrentWindow().destroy();
  } catch {
    closeConfirmed = false;
  }
}

async function hideToTray() {
  try {
    await getCurrentWindow().hide();
    tray.setHidden(true);
  } catch {
    // The window stays open if it cannot be hidden.
  }
}

async function quitFromTray() {
  if (!config.dirty) {
    await tray.quit();
    return;
  }
  quitAfterClose = true;
  tray.setHidden(false);
  const window = getCurrentWindow();
  await window.unminimize().catch(() => undefined);
  await window.show().catch(() => undefined);
  await window.setFocus().catch(() => undefined);
  closeAsked.value = true;
}

function cancelClose() {
  closeAsked.value = false;
  quitAfterClose = false;
}

async function saveAndClose() {
  if (await save()) await closeWindow();
}

watch(() => tray.quitRequested, () => void quitFromTray());

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  try {
    unlistenClose = await getCurrentWindow().onCloseRequested((event) => {
      if (closeConfirmed) return;
      if (tray.hideOnClose) {
        event.preventDefault();
        void hideToTray();
        return;
      }
      if (!config.dirty) return;
      event.preventDefault();
      closeAsked.value = true;
    });
  } catch {
    // Outside Tauri there is no window to guard.
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  unlistenClose?.();
});
</script>

<template>
  <div v-if="config.dirty" class="unsaved-bar" role="status">
    <span class="dot" />
    <span class="text"><strong>{{ t("shell.unsaved.title") }}</strong> {{ t("shell.unsaved.body") }}</span>
    <span class="shortcut">{{ t("shell.unsaved.shortcut", { keys: "Ctrl+S" }) }}</span>
    <n-button type="primary" size="small" :loading="saving" :disabled="saving || !daemon.canWrite" @click="save">
      {{ t("common.save") }}
    </n-button>
  </div>

  <n-modal
    :show="closeAsked"
    :mask-closable="false"
    :close-on-esc="true"
    @update:show="(show: boolean) => { if (!show) cancelClose(); }"
  >
    <div class="close-dialog" role="dialog" aria-modal="true" aria-labelledby="unsaved-close-title">
      <span class="warn-icon"><AlertTriangle :size="22" /></span>
      <h2 id="unsaved-close-title" class="close-title">{{ t("shell.unsaved.closeTitle") }}</h2>
      <p class="close-body">{{ t("shell.unsaved.closeBody") }}</p>
      <div class="close-actions">
        <n-button quaternary type="primary" @click="cancelClose">{{ t("common.cancel") }}</n-button>
        <n-button secondary @click="closeWindow">{{ t("shell.unsaved.closeDiscard") }}</n-button>
        <n-button type="primary" :loading="saving" :disabled="saving || !daemon.canWrite" @click="saveAndClose">
          {{ t("shell.unsaved.closeSave") }}
        </n-button>
      </div>
    </div>
  </n-modal>
</template>

<style scoped>
.unsaved-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) 14px;
  padding: 10px 12px 10px 18px;
  border-radius: 18px;
  background: color-mix(in srgb, var(--warning) 16%, var(--md-surface-container));
}
.dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--warning);
}
.text {
  flex: 1;
  min-width: 200px;
  font-size: 14px;
}
.shortcut {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
.close-dialog {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: min(440px, calc(100vw - 32px));
  box-sizing: border-box;
  padding: 24px;
  border-radius: 28px;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
}
.warn-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--warning) 18%, transparent);
  color: var(--warning);
}
.close-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 22px;
  font-weight: 600;
}
.close-body {
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: var(--md-on-surface-variant);
}
.close-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
</style>
