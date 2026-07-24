import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { useConfigStore } from "@/stores/config";

/**
 * Thermal-alert helpers (new feature beyond Slint parity). State lives in the
 * config mirror; this store exposes computed views for the Settings page
 * status indicator.
 */
export const useThermalStore = defineStore("thermal", () => {
  const config = useConfigStore();
  const cpuTemp = ref<number | null>(null);
  const gpuTemp = ref<number | null>(null);

  const cpuActive = computed(
    () => config.thermalAlert.cpu.enabled && cpuTemp.value !== null &&
      cpuTemp.value >= config.thermalAlert.cpu.threshold,
  );
  const gpuActive = computed(
    () => config.thermalAlert.gpu.enabled && gpuTemp.value !== null &&
      gpuTemp.value >= config.thermalAlert.gpu.threshold,
  );

  /** One of: "active" (alerting), "monitoring" (armed), "disabled". */
  const status = computed<"active" | "monitoring" | "disabled">(() => {
    if (cpuActive.value || gpuActive.value) return "active";
    if (config.thermalAlert.cpu.enabled || config.thermalAlert.gpu.enabled) return "monitoring";
    return "disabled";
  });

  function setTemps(cpu: number | null, gpu: number | null) {
    cpuTemp.value = cpu;
    gpuTemp.value = gpu;
  }

  return { cpuTemp, gpuTemp, cpuActive, gpuActive, status, setTemps };
});
