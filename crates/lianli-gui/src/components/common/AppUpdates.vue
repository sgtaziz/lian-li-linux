<script setup lang="ts">
import { ref } from "vue";
import { useDialog } from "naive-ui";
import { invoke } from "@/utils/backendText";
import { useConfigStore } from "@/stores/config";
import { useTranslate } from "@/i18n";

const t = useTranslate();
const config = useConfigStore();
const dialog = useDialog();
const busy = ref(false);
const error = ref("");
const update = ref<{ installed: string; available: string | null } | null>(null);

async function check() {
  busy.value = true;
  error.value = "";
  update.value = null;
  try { update.value = await invoke("check_package_update"); }
  catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}

function confirmUpdate() {
  dialog.warning({
    title: t("settings.updates.install"),
    content: t("settings.updates.confirm"),
    positiveText: t("settings.updates.install"),
    negativeText: t("settings.updates.cancel"),
    onPositiveClick: async () => {
      if (config.dirty || busy.value) return;
      busy.value = true;
      error.value = "";
      try { await invoke("start_package_update"); }
      catch (e) { error.value = String(e); }
      finally { busy.value = false; }
    },
  });
}
</script>

<template>
  <section class="card">
    <div class="section-head">
      <h2 class="section-title">{{ t("settings.updates.title") }}</h2>
      <n-button size="small" :loading="busy" :disabled="busy" @click="check">{{ t("settings.updates.check") }}</n-button>
    </div>
    <p class="hint">{{ t("settings.updates.hint") }}</p>
    <div v-if="update" class="update-status muted" aria-live="polite">
      <p>{{ t("settings.updates.installed", { version: update.installed }) }}</p>
      <p>{{ update.available ? t("settings.updates.available", { version: update.available }) : t("settings.updates.current") }}</p>
    </div>
    <p v-if="config.dirty" class="hint">{{ t("settings.updates.saveFirst") }}</p>
    <n-alert v-if="error" type="error" class="update-error">{{ error }}</n-alert>
    <div class="update-actions">
      <n-button size="small" :disabled="busy || config.dirty || !update?.available" @click="confirmUpdate">{{ t("settings.updates.install") }}</n-button>
    </div>
  </section>
</template>

<style scoped>
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-3);
}
.section-title {
  margin: 0;
}
.hint {
  color: var(--text-muted);
  font-size: var(--font-size-sm);
  margin: var(--space-2) 0 0;
}
.update-status {
  font-size: var(--font-size-sm);
  margin-top: var(--space-3);
}
.update-status p {
  margin: var(--space-2) 0 0;
}
.update-error {
  margin-top: var(--space-3);
}
.update-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--space-3);
}
</style>
