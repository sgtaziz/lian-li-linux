<script setup lang="ts">
import { computed } from "vue";
import SegmentedControl from "@/components/common/SegmentedControl.vue";
import { useThemeStore, type ThemePreference } from "@/stores/theme";
import { useTrayStore, type TrayIconStyle } from "@/stores/tray";
import { DEFAULT_ACCENT } from "@/theme/palette";
import { useTranslate } from "@/i18n";

const theme = useThemeStore();
const tray = useTrayStore();
const t = useTranslate();

const ACCENT_SWATCHES = [
  "#3584e4",
  "#2190a4",
  "#3a944a",
  "#c88800",
  "#ed5b00",
  "#e62d42",
  "#d56199",
  "#9141ac",
  "#6f8396",
];

const styleOptions = computed(() => [
  { value: "system" as const, label: t("settings.appearance.system") },
  { value: "light" as const, label: t("settings.appearance.light") },
  { value: "dark" as const, label: t("settings.appearance.dark") },
]);

const trayIconOptions = computed(() => [
  { value: "auto" as const, label: t("settings.tray.iconAuto") },
  { value: "light" as const, label: t("settings.tray.iconLight") },
  { value: "dark" as const, label: t("settings.tray.iconDark") },
]);

const followsDesktop = computed(() => theme.accentMode === "system");
const desktopSwatch = computed(() => theme.desktopAccent ?? DEFAULT_ACCENT);

const roles = computed(() => [
  { name: t("settings.appearance.rolePrimary"), bg: theme.palette.primary, fg: theme.palette.onPrimary },
  { name: t("settings.appearance.rolePrimaryContainer"), bg: theme.palette.primaryContainer, fg: theme.palette.onPrimaryContainer },
  { name: t("settings.appearance.roleSecondary"), bg: theme.palette.secondaryContainer, fg: theme.palette.onSecondaryContainer },
  { name: t("settings.appearance.roleTertiary"), bg: theme.palette.tertiaryContainer, fg: theme.palette.onTertiaryContainer },
  { name: t("settings.appearance.roleSurface"), bg: theme.palette.surfaceContainerHigh, fg: theme.palette.onSurface },
]);

function onCustomPicked(value: string) {
  theme.setCustomAccent(value.slice(0, 7));
}
</script>

<template>
  <section class="card appearance">
    <h2 class="section-title">{{ t("settings.appearance.title") }}</h2>

    <div class="row">
      <div class="row-text">
        <span class="row-title">{{ t("settings.appearance.style") }}</span>
        <span class="row-hint">{{ t("settings.appearance.styleHint") }}</span>
      </div>
      <SegmentedControl
        :model-value="theme.preference"
        :options="styleOptions"
        :label="t('settings.appearance.style')"
        @update:model-value="(v: ThemePreference) => theme.setPreference(v)"
      />
    </div>

    <div class="divider" />

    <div class="row">
      <div class="row-text">
        <span class="row-title">{{ t("settings.tray.closeToTray") }}</span>
        <span class="row-hint">{{ tray.available ? t("settings.tray.closeToTrayHint") : t("settings.tray.unavailable") }}</span>
      </div>
      <n-switch
        :value="tray.closeToTray"
        :disabled="!tray.available"
        :aria-label="t('settings.tray.closeToTray')"
        @update:value="(v: boolean) => tray.setCloseToTray(v)"
      />
    </div>

    <div class="row">
      <div class="row-text">
        <span class="row-title">{{ t("settings.tray.icon") }}</span>
        <span class="row-hint">{{ t("settings.tray.iconHint") }}</span>
      </div>
      <SegmentedControl
        :model-value="tray.iconStyle"
        :options="trayIconOptions"
        :label="t('settings.tray.icon')"
        :disabled="!tray.available || !tray.closeToTray"
        @update:model-value="(v: TrayIconStyle) => tray.setIconStyle(v)"
      />
    </div>

    <div class="divider" />

    <div class="accent">
      <span class="row-title">{{ t("settings.appearance.accent") }}</span>

      <button
        type="button"
        class="option"
        :class="{ selected: followsDesktop }"
        :aria-pressed="followsDesktop"
        @click="theme.setAccentMode('system')"
      >
        <span class="radio"><span /></span>
        <span class="option-text">
          <span class="option-title">{{ t("settings.appearance.followDesktop") }}</span>
          <span class="row-hint">
            {{ theme.desktopAccent ? t("settings.appearance.detected", { color: theme.desktopAccent }) : t("settings.appearance.notShared") }}
          </span>
        </span>
        <span class="swatch large" :style="{ background: desktopSwatch }" />
      </button>

      <div class="option custom" :class="{ selected: !followsDesktop }">
        <button
          type="button"
          class="custom-head"
          :aria-pressed="!followsDesktop"
          @click="theme.setAccentMode('custom')"
        >
          <span class="radio"><span /></span>
          <span class="option-title">{{ t("settings.appearance.custom") }}</span>
        </button>
        <div class="swatches">
          <button
            v-for="hex in ACCENT_SWATCHES"
            :key="hex"
            type="button"
            class="swatch"
            :class="{ active: !followsDesktop && theme.customAccent === hex }"
            :style="{ background: hex }"
            :aria-label="t('settings.appearance.useColor', { color: hex })"
            :title="hex"
            @click="theme.setCustomAccent(hex)"
          />
          <n-color-picker
            class="picker"
            size="small"
            :value="theme.customAccent"
            :modes="['hex']"
            :show-alpha="false"
            @update:value="onCustomPicked"
          >
            <template #trigger="{ onClick, ref: setTriggerRef }">
              <button
                :ref="setTriggerRef"
                type="button"
                class="picker-trigger"
                :aria-label="t('settings.appearance.pickColor')"
                @click="onClick"
              >
                <span class="swatch small" :style="{ background: theme.customAccent }" />
                <span class="picker-hex">{{ theme.customAccent }}</span>
              </button>
            </template>
          </n-color-picker>
        </div>
      </div>
    </div>

    <div class="preview">
      <span class="row-hint">{{ t("settings.appearance.preview") }}</span>
      <div class="roles">
        <div v-for="role in roles" :key="role.name" class="role" :style="{ background: role.bg, color: role.fg }">
          {{ role.name }}
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.appearance {
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.appearance .section-title {
  margin: 0;
}
.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}
.row-text,
.option-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.row-text {
  flex: 1 1 320px;
}
.row-title {
  font-size: 15px;
  font-weight: 600;
}
.row-hint {
  font-size: 13px;
  color: var(--md-on-surface-variant);
}
.divider {
  height: 1px;
  background: var(--md-outline-variant);
}
.accent {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.option {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px var(--space-4);
  border-radius: 18px;
  border: 1px solid var(--md-outline-variant);
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  text-align: left;
}
button.option {
  cursor: pointer;
}
.option.selected {
  border-color: var(--md-primary);
  background: var(--md-surface-container-high);
}
.option-text {
  flex: 1;
}
.option-title {
  font-size: 14px;
  font-weight: 600;
}
.custom {
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-3);
}
.custom-head {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.radio {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  box-sizing: border-box;
  border-radius: 50%;
  border: 2px solid var(--md-outline);
}
.radio span {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}
.selected .radio {
  border-color: var(--md-primary);
}
.selected .radio span {
  background: var(--md-primary);
}
.swatches {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding-left: 34px;
}
.swatch {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  padding: 0;
  border: 0;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.15);
  cursor: pointer;
}
.swatch.large {
  width: 28px;
  height: 28px;
  cursor: inherit;
}
.swatch.active {
  box-shadow: 0 0 0 2px var(--md-surface-container-high), 0 0 0 4px var(--md-on-surface);
}
.swatch:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 3px;
}
.picker {
  flex: 0 0 auto;
}
.swatch.small {
  width: 22px;
  height: 22px;
  cursor: inherit;
}
.picker-trigger {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  height: 32px;
  padding: 0 var(--space-3) 0 5px;
  border: 1px solid var(--md-outline);
  border-radius: 16px;
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  cursor: pointer;
}
.picker-trigger:hover {
  background: color-mix(in srgb, var(--md-on-surface) 8%, transparent);
}
.picker-trigger:focus-visible {
  outline: 2px solid var(--md-primary);
  outline-offset: 2px;
}
.picker-hex {
  font-family: var(--font-mono);
  font-size: 12px;
}
.preview {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.roles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: var(--space-2);
}
.role {
  display: flex;
  align-items: flex-end;
  height: 72px;
  box-sizing: border-box;
  padding: 10px 12px;
  border-radius: 16px;
  border: 1px solid var(--md-outline-variant);
  font-size: var(--font-size-sm);
  font-weight: 600;
}
</style>
