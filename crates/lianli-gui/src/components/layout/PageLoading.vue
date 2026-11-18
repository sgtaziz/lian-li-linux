<script setup lang="ts">
import { useTranslate } from "@/i18n";

const t = useTranslate();
</script>

<template>
  <div class="page-loading" role="status" :aria-label="t('shell.loading')">
    <div class="block hero" />
    <div class="row">
      <div class="block tile" />
      <div class="block tile" />
      <div class="block tile" />
    </div>
    <div class="row">
      <div class="block card" />
      <div class="block card" />
    </div>
    <span class="label">{{ t("shell.loading") }}</span>
  </div>
</template>

<style scoped>
.page-loading {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.row {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-4);
}
.block {
  border-radius: 28px;
  background: var(--md-surface-container-low);
  animation: loading-pulse 1.4s ease-in-out infinite;
}
.hero {
  height: 180px;
  border-radius: 32px;
}
.tile {
  height: 84px;
}
.card {
  height: 150px;
}
.label {
  font-size: var(--font-size-sm);
  color: var(--md-on-surface-variant);
}
@keyframes loading-pulse {
  50% {
    opacity: 0.55;
  }
}
</style>
