<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Plus, Sparkles, AlertTriangle } from "lucide-vue-next";
import { useConfigStore } from "@/stores/config";
import { useDevicesStore } from "@/stores/devices";
import { useLcdStore } from "@/stores/lcd";
import { useMessage } from "naive-ui";
import { PIXEL_CLEANER_DURATION_OPTIONS } from "@/constants";
import LcdConfigCard from "@/components/lcd/LcdConfigCard.vue";

const config = useConfigStore();
const devices = useDevicesStore();
const lcd = useLcdStore();
const message = useMessage();

const entries = computed(() => config.config.lcds);

const showCleanerModal = ref(false);
const cleanerTarget = ref<string | null>("ALL");
const cleanerDuration = ref(30);

watch(showCleanerModal, (shown) => {
  if (shown && lcd.cleaningActive) {
    cleanerTarget.value = lcd.cleaningDeviceId ?? "ALL";
    cleanerDuration.value = lcd.cleaningDurationMinutes;
  }
});

const durationOptions = PIXEL_CLEANER_DURATION_OPTIONS;

const targetOptions = computed(() => {
  const opts = [{ label: "All Detected LCDs", value: "ALL" }];
  for (const d of devices.lcdDevices) {
    opts.push({
      label: d.name + (d.serial ? ` (${d.serial})` : ""),
      value: d.device_id,
    });
  }
  return opts;
});

async function startCleaner() {
  const devId = cleanerTarget.value === "ALL" ? null : cleanerTarget.value;
  try {
    await lcd.startPixelClean(devId, cleanerDuration.value);
    message.success(
      `Pixel cleaner started for ${cleanerDuration.value} minutes at 75% brightness`,
    );
    showCleanerModal.value = false;
  } catch (err: any) {
    message.error(`Failed to start pixel cleaner: ${err}`);
  }
}

async function stopCleaner() {
  try {
    const devId =
      lcd.cleaningDeviceId ??
      (cleanerTarget.value === "ALL" ? null : cleanerTarget.value);
    await lcd.stopPixelClean(devId);
    message.info("Pixel cleaner stopped; previous LCD display restored");
  } catch (err: any) {
    message.error(`Failed to stop pixel cleaner: ${err}`);
  }
}

function addLcd() {
  const first = devices.lcdDevices[0];
  config.addLcd({
    serial: first?.serial ?? null,
    index: first?.serial ? undefined : 0,
    type: "image",
    path: null,
    fps: null,
    orientation: 0,
    rgb: null,
  });
}
</script>

<template>
  <div class="page lcd-page">
    <div class="page-head">
      <n-button
        size="small"
        type="primary"
        @click="addLcd"
        :disabled="!devices.lcdDevices.length"
      >
        <template #icon><Plus :size="15" /></template>
        Add LCD
      </n-button>
      <n-button
        size="small"
        secondary
        :type="lcd.cleaningActive ? 'error' : 'warning'"
        @click="showCleanerModal = true"
        :disabled="!devices.lcdDevices.length"
      >
        <template #icon><Sparkles :size="15" /></template>
        {{
          lcd.cleaningActive
            ? "Cleaner Active (" + lcd.formattedRemaining + ")"
            : "Pixel Cleaner"
        }}
      </n-button>
      <span v-if="!devices.lcdDevices.length" class="muted">
        No LCD devices detected.
      </span>
    </div>

    <!-- Active Cleaner Banner -->
    <div v-if="lcd.cleaningActive" class="card cleaner-banner">
      <div class="cleaner-banner-text">
        <Sparkles class="cleaner-icon" :size="20" />
        <div>
          <div class="cleaner-title">Pixel Conditioning In Progress</div>
          <div class="muted">
            Cycling dynamic static, rapid strobing, and solid color washes at
            75% brightness. Time remaining:
            <strong>{{ lcd.formattedRemaining }}</strong>
          </div>
        </div>
      </div>
      <n-button size="small" type="error" @click="stopCleaner">
        Stop & Restore
      </n-button>
    </div>

    <!-- Pixel Cleaner Modal -->
    <n-modal
      v-model:show="showCleanerModal"
      preset="card"
      title="LCD Pixel Conditioner / Retention Cleaner"
      style="max-width: 540px"
    >
      <div class="cleaner-modal-content">
        <div class="info-box">
          <AlertTriangle :size="18" class="warning-icon" />
          <span>
            Exercises LCD panels with dynamic static noise, 10 Hz black/white
            strobing, and solid color washes at 75% brightness. This dissolves
            trapped DC bias ions and relieves stubborn image retention or
            polarity inversion flicker.
          </span>
        </div>

        <div class="form-row">
          <label class="muted">Target Display</label>
          <n-select
            v-model:value="cleanerTarget"
            :options="targetOptions"
            :disabled="lcd.cleaningActive"
          />
        </div>

        <div class="form-row">
          <label class="muted">Duration</label>
          <n-select
            v-model:value="cleanerDuration"
            :options="durationOptions"
            :disabled="lcd.cleaningActive"
          />
        </div>

        <div v-if="lcd.cleaningActive" class="status-box">
          Conditioning active. Time remaining:
          <strong>{{ lcd.formattedRemaining }}</strong>
        </div>

        <div class="modal-actions">
          <n-button
            v-if="!lcd.cleaningActive"
            type="warning"
            @click="startCleaner"
          >
            <template #icon><Sparkles :size="16" /></template>
            Start Conditioning
          </n-button>
          <n-button v-else type="error" @click="stopCleaner">
            Stop & Restore Display
          </n-button>
          <n-button quaternary @click="showCleanerModal = false"
            >Close</n-button
          >
        </div>
      </div>
    </n-modal>

    <LcdConfigCard
      v-for="(entry, i) in entries"
      :key="i"
      :entry="entry"
      :index="i"
    />

    <div
      v-if="!entries.length && devices.lcdDevices.length"
      class="card empty muted"
    >
      No LCD configurations. Click "Add LCD" to create one.
    </div>
  </div>
</template>

<style scoped>
.lcd-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  max-width: 1100px;
}
.page-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.cleaner-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-left: 4px solid var(--warning);
  padding: var(--space-3) var(--space-4);
  background: rgba(245, 158, 11, 0.08);
}
.cleaner-banner-text {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.cleaner-icon {
  color: var(--warning);
  animation: spin-pulse 2s infinite ease-in-out;
}
@keyframes spin-pulse {
  0%,
  100% {
    transform: scale(1);
    opacity: 0.8;
  }
  50% {
    transform: scale(1.15);
    opacity: 1;
  }
}
.cleaner-title {
  font-weight: 600;
  margin-bottom: 2px;
}
.cleaner-modal-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.info-box {
  display: flex;
  gap: var(--space-3);
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: rgba(245, 158, 11, 0.1);
  border: 1px solid rgba(245, 158, 11, 0.2);
  font-size: var(--font-size-sm);
  color: var(--text-2);
}
.warning-icon {
  color: var(--warning);
  flex-shrink: 0;
  margin-top: 2px;
}
.form-row {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.status-box {
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: rgba(79, 158, 255, 0.1);
  border: 1px solid rgba(79, 158, 255, 0.2);
  text-align: center;
}
.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-2);
}
.empty {
  padding: var(--space-6);
  text-align: center;
}
</style>
