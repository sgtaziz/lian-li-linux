<script setup lang="ts">
withDefaults(
  defineProps<{
    color?: "success" | "danger" | "warning" | "muted";
    pulse?: boolean;
  }>(),
  { color: "muted", pulse: false },
);
</script>

<template>
  <span class="status-dot" :class="[color, { pulse }]" />
</template>

<style scoped>
.status-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.success {
  background: var(--success);
  box-shadow: 0 0 6px var(--success);
}
.danger {
  background: var(--danger);
  box-shadow: 0 0 6px var(--danger);
}
.warning {
  background: var(--warning);
  box-shadow: 0 0 6px var(--warning);
}
.muted {
  background: var(--text-muted);
}
.pulse {
  animation: dot-pulse 1.4s ease-in-out infinite;
}
@keyframes dot-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.4;
  }
}
</style>
