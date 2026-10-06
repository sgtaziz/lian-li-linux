<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useDaemonStore } from "@/stores/daemon";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdNamesStore } from "@/stores/lcdNames";
import { lcdDeviceLabels, lcdEntryKey, resolveLcdDevice } from "@/utils/lcdSelection";
import type { LcdConfig, LcdTemplate } from "@/types";

const props = defineProps<{ lcds: LcdConfig[]; templates: LcdTemplate[] }>();
const emit = defineEmits<{ inProgress: [active: boolean] }>();
interface Result { import_id: string; lcds: LcdConfig[]; templates: LcdTemplate[] }
interface Status { id: string; active: boolean; result: Result | null; error: string | null }
const daemon = useDaemonStore();
const config = useConfigStore();
const devices = useDevicesStore();
const lcdNames = useLcdNamesStore();

function screenName(entry: LcdConfig): string {
  const device = resolveLcdDevice(entry, devices.lcdDevices);
  const label = device ? lcdDeviceLabels(devices.lcdDevices).get(device.device_id) ?? device.name : null;
  return lcdNames.names[lcdEntryKey(entry)] || label || "Offline screen";
}

const incomplete = computed(() =>
  props.lcds.flatMap((entry) => {
    if (["image", "gif", "video"].includes(entry.type) && !entry.path) return [`${screenName(entry)}: choose a file`];
    if (entry.type === "custom" && !entry.template_id) return [`${screenName(entry)}: choose a template`];
    return [];
  }),
);

function readableError(message: string): string {
  return message.replace(/LCD\[serial:([^\]]+)\]/g, (match, serial: string) => {
    const entry = props.lcds.find((candidate) => candidate.serial?.replace(/^hid:/, "") === serial.replace(/^hid:/, ""));
    return entry ? screenName(entry) : match;
  });
}

const status = ref<Status | null>(null);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const confirmed = ref(false);
const pending = ref("");
const stagedId = ref("");
let revision = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let pendingChecks = 0;

function reset() {
  revision++;
  clearTimeout(timer);
  status.value = null;
  pending.value = "";
  stagedId.value = "";
  pendingChecks = 0;
  busy.value = false;
  error.value = "";
  notice.value = "";
  confirmed.value = false;
}
watch([() => daemon.connected, () => daemon.info?.instance_id], reset);
watch(
  () =>
    busy.value ||
    !!pending.value ||
    !!status.value?.active ||
    (!!status.value?.result && status.value.id !== stagedId.value),
  (active) => emit("inProgress", active),
  { immediate: true },
);
onUnmounted(reset);

async function check() {
  if (busy.value) return;
  const current = revision;
  busy.value = true;
  clearTimeout(timer);
  try {
    const value = await invoke<Status | null>("managed_import_status");
    if (current !== revision) return;
    if (value && (!/^[a-f0-9]{32}$/i.test(value.id) || typeof value.active !== "boolean")) throw new Error("Invalid import progress");
    if (pending.value && value?.id !== pending.value) {
      notice.value = "The submitted worker has not reported yet. Check progress again before retrying.";
      if (++pendingChecks < 30) timer = setTimeout(() => void check(), 1000);
      return;
    }
    status.value = value;
    pending.value = "";
    pendingChecks = 0;
    error.value = value?.error ? `Previous import failed: ${value.error}` : "";
    notice.value = value?.active ? "Copying and checking media. You can close this page while it finishes."
      : value?.result ? "Copies are ready. Review and stage them below, then Save to apply."
        : value ? "Import did not complete. Inspect the reported error before retrying." : "No import is recorded for this desktop session.";
    if (value?.active) timer = setTimeout(() => void check(), 1000);
  } catch (reason) {
    if (current === revision) error.value = String(reason);
  } finally {
    if (current === revision) busy.value = false;
  }
}

async function start() {
  if (busy.value || status.value?.active || pending.value || !daemon.info || config.imported || incomplete.value.length) return;
  const current = ++revision;
  clearTimeout(timer);
  busy.value = true;
  error.value = "";
  status.value = null;
  confirmed.value = false;
  notice.value = "Submitting managed copy…";
  try {
    const id = await invoke<string>("managed_import_start", { instance: daemon.info.instance_id, lcds: props.lcds, templates: props.templates });
    if (current !== revision) return;
    pending.value = id;
    notice.value = "Submitted. A desktop authentication prompt may appear.";
    timer = setTimeout(() => void check(), 1000);
  } catch (reason) {
    if (current === revision) { error.value = String(reason); notice.value = "Submission was not confirmed. Check import progress before retrying."; }
  } finally {
    if (current === revision) busy.value = false;
  }
}

async function stage() {
  if (busy.value || !confirmed.value || !status.value?.result || !daemon.info) return;
  const current = revision;
  const instance = daemon.info.instance_id;
  busy.value = true;
  try {
    const result = await invoke<Result>("managed_import_result", { instance, id: status.value.id });
    if (current !== revision) return;
    config.stageImportedMedia(result.lcds, result.templates, instance);
    stagedId.value = status.value?.id ?? "";
    confirmed.value = false;
    notice.value = "Copied paths are staged. Save the configuration to apply them, or Reload to discard these drafts.";
  } catch (reason) {
    if (current === revision) error.value = String(reason);
  } finally {
    if (current === revision) busy.value = false;
  }
}
</script>

<template>
  <n-card title="Managed media copies" size="small">
    <p>Copy selected media and template assets into daemon-managed storage. Originals stay unchanged. Native imports require authorization. Distrobox supports either daemon mode under the same account and inside the same box.</p>
    <n-space>
      <n-button :disabled="busy || !daemon.connected || !lcds.length || !!incomplete.length || !!config.imported || !!status?.active || !!pending" @click="start">Copy into managed storage</n-button>
      <n-button :disabled="busy" @click="check">Check import progress</n-button>
    </n-space>
    <p v-if="notice" role="status">{{ notice }}</p>
    <n-alert v-if="incomplete.length" type="warning" title="Finish these screens before copying">
      <ul class="incomplete"><li v-for="item in incomplete" :key="item">{{ item }}</li></ul>
    </n-alert>
    <n-alert v-if="error" type="error">{{ readableError(error) }}</n-alert>
    <template v-if="status?.result">
      <p>{{ status.result.lcds.length }} LCD selections · {{ status.result.templates.length }} templates copied</p>
      <ul><li v-for="(lcd, index) in status.result.lcds" :key="index">LCD {{ index + 1 }}: {{ lcd.path ?? lcd.template_id ?? lcd.type }}</li></ul>
      <n-checkbox v-model:checked="confirmed" :disabled="busy || !!config.imported">Replace my current LCD drafts and matching templates with this copied selection.</n-checkbox>
      <n-button :disabled="busy || !confirmed || !!config.imported || !daemon.connected" @click="stage">Stage copied paths</n-button>
    </template>
  </n-card>
</template>

<style scoped>
p { margin: var(--space-2) 0; }
li { overflow-wrap: anywhere; }
.incomplete { margin: 0; padding-left: var(--space-4); }
</style>
