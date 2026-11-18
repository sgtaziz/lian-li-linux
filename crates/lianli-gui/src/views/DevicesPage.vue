<script setup lang="ts">
import { computed } from "vue";
import { useDevicesStore, type DisplaySwitchError } from "@/stores/devices";
import DeviceCard from "@/components/devices/DeviceCard.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import { Cpu, Fan, Monitor, Palette, Thermometer } from "lucide-vue-next";
import { useThermalStore } from "@/stores/thermal";
import { useTranslate } from "@/i18n";

const devices = useDevicesStore();
const thermal = useThermalStore();
const t = useTranslate();

const empty = computed(() => devices.displayCards.length === 0);

const counts = computed(() => {
  const list = devices.visible;
  return {
    total: list.length,
    cooling: list.filter((d) => d.has_fan || d.has_pump).length,
    lighting: list.filter((d) => d.has_rgb).length,
    screens: list.filter((d) => d.has_lcd).length,
  };
});

function tempTone(celsius: number): string {
  if (celsius < 60) return "cool";
  if (celsius < 80) return "warm";
  return "hot";
}

// Readings exist only while thermal alerts poll sensors; the page adds no sensor polling of its own.
const temps = computed(() =>
  [
    { id: "cpu", label: t("devices.hero.cpu"), value: thermal.cpuTemp, icon: Cpu },
    { id: "gpu", label: t("devices.hero.gpu"), value: thermal.gpuTemp, icon: Thermometer },
  ].filter((temp): temp is typeof temp & { value: number } => temp.value !== null),
);

function switchErrorText(error: DisplaySwitchError): string {
  return "message" in error ? error.message : t("devices.page.switchTimeout", { name: error.timedOutDevice });
}

</script>

<template>
  <div class="page devices-page">
    <n-alert v-for="(error, id) in devices.displaySwitchErrors" :key="id" type="error" closable @close="delete devices.displaySwitchErrors[id]">{{ switchErrorText(error) }}</n-alert>

    <section class="summary">
      <div class="summary-count">
        <span class="count">{{ counts.total }}</span>
        <span class="count-label">{{ t("devices.hero.connected") }}</span>
      </div>
      <div class="chips">
        <span class="chip chip-cooling"><Fan :size="15" />{{ counts.cooling }} {{ t("devices.hero.cooling") }}</span>
        <span class="chip chip-lighting"><Palette :size="15" />{{ counts.lighting }} {{ t("devices.hero.lighting") }}</span>
        <span class="chip chip-screens"><Monitor :size="15" />{{ counts.screens }} {{ t("devices.hero.screens") }}</span>
      </div>
      <div v-if="temps.length" class="temps">
        <span v-for="temp in temps" :key="temp.id" class="chip temp-chip">
          <component :is="temp.icon" :size="15" />{{ temp.label }}
          <strong>{{ Math.round(temp.value) }}°C</strong>
          <span class="tone" :class="`tone-${tempTone(temp.value)}`" />
        </span>
      </div>
    </section>

    <EmptyState v-if="empty" kind="devices" />

    <template v-else>
      <h2 class="section-title list-title">{{ t("devices.hero.hardware") }}</h2>
      <div class="device-list">
        <DeviceCard
          v-for="d in devices.displayCards"
          :key="d.device_id"
          :device="d"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.summary {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3) var(--space-6);
  padding: var(--space-3) 20px;
  border-radius: 20px;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
}
.summary-count {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
}
.count {
  font-family: var(--font-display);
  font-size: 28px;
  font-weight: 600;
  line-height: 1;
}
.count-label {
  font-family: var(--font-display);
  font-size: 16px;
  font-weight: 500;
}
.chips,
.temps {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.temps {
  margin-left: auto;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  border-radius: 12px;
  font-size: var(--font-size-sm);
  font-weight: 600;
  white-space: nowrap;
}
.chip-cooling {
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}
.chip-lighting {
  background: var(--md-tertiary-container);
  color: var(--md-on-tertiary-container);
}
.chip-screens,
.temp-chip {
  background: var(--md-surface);
  color: var(--md-on-surface);
}
.tone {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--md-outline-variant);
}
.tone-cool { background: var(--temp-cool); }
.tone-warm { background: var(--temp-warm); }
.tone-hot { background: var(--temp-hot); }
.list-title {
  margin: var(--space-4) 0 0;
}
.device-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 440px), 1fr));
  gap: var(--space-4);
}
</style>
