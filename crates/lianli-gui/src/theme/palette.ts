import type { GlobalThemeOverrides } from "naive-ui";

export const DEFAULT_ACCENT = "#3584e4";

export interface Palette {
  primary: string;
  onPrimary: string;
  primaryContainer: string;
  onPrimaryContainer: string;
  secondary: string;
  secondaryContainer: string;
  onSecondaryContainer: string;
  tertiary: string;
  tertiaryContainer: string;
  onTertiaryContainer: string;
  surface: string;
  surfaceContainerLow: string;
  surfaceContainer: string;
  surfaceContainerHigh: string;
  surfaceContainerHighest: string;
  onSurface: string;
  onSurfaceVariant: string;
  outline: string;
  outlineVariant: string;
  inverseSurface: string;
  inverseOnSurface: string;
  primaryHover: string;
  primaryPressed: string;
  success: string;
  warning: string;
  danger: string;
  tempCool: string;
  tempWarm: string;
  tempHot: string;
}

const HEX = /^#?([0-9a-f]{6})$/i;

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && HEX.test(value);
}

export function normalizeHex(value: unknown): string | null {
  const match = typeof value === "string" ? HEX.exec(value.trim()) : null;
  return match ? `#${match[1].toLowerCase()}` : null;
}

function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const linear = (channel: number) => {
    const v = channel / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear((n >> 16) & 255) + 0.7152 * linear((n >> 8) & 255) + 0.0722 * linear(n & 255);
}

function toneUnderWhiteText(h: number, s: number, l: number): number {
  let tone = l;
  while (tone > 0 && 1.05 / (luminance(hsl(h, s, tone)) + 0.05) < 4.5) tone -= 1;
  return tone;
}

function toHsl(hex: string): [number, number, number] {
  const match = HEX.exec(hex);
  const n = parseInt(match ? match[1] : DEFAULT_ACCENT.slice(1), 16);
  const r = ((n >> 16) & 255) / 255;
  const g = ((n >> 8) & 255) / 255;
  const b = (n & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l * 100];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  h *= 60;
  return [h, s * 100, l * 100];
}

function hsl(h: number, s: number, l: number): string {
  const hue = (((h % 360) + 360) % 360) / 360;
  const sat = Math.min(100, Math.max(0, s)) / 100;
  const light = Math.min(100, Math.max(0, l)) / 100;
  const q = light < 0.5 ? light * (1 + sat) : light + sat - light * sat;
  const p = 2 * light - q;
  const channel = (t: number) => {
    const x = t < 0 ? t + 1 : t > 1 ? t - 1 : t;
    const v = x < 1 / 6 ? p + (q - p) * 6 * x : x < 1 / 2 ? q : x < 2 / 3 ? p + (q - p) * (2 / 3 - x) * 6 : p;
    return Math.round(v * 255).toString(16).padStart(2, "0");
  };
  return `#${channel(hue + 1 / 3)}${channel(hue)}${channel(hue - 1 / 3)}`;
}

/**
 * Builds tonal roles from one accent. Primary keeps the accent hue, secondary is a
 * muted version of it, tertiary is rotated 60 degrees, and neutrals carry a faint tint.
 * Lightness values follow Material 3 tone steps so text roles keep readable contrast.
 */
export function buildPalette(accent: string, dark: boolean): Palette {
  const [h, s0] = toHsl(accent);
  const p = s0 < 22 ? Math.max(s0, 14) : Math.min(Math.max(s0, 50), 88);
  const n = Math.min(p * 0.16, 9);
  const nv = Math.min(p * 0.22, 14);
  const sec = Math.min(p * 0.38, 30);
  const th = h + 60;
  const tp = Math.min(p, 60);

  if (dark) {
    return {
      primary: hsl(h, p, 76),
      onPrimary: hsl(h, p, 15),
      primaryContainer: hsl(h, p * 0.7, 26),
      onPrimaryContainer: hsl(h, p, 91),
      secondary: hsl(h, sec, 75),
      secondaryContainer: hsl(h, sec, 25),
      onSecondaryContainer: hsl(h, sec + 10, 91),
      tertiary: hsl(th, tp * 0.8, 75),
      tertiaryContainer: hsl(th, tp * 0.55, 24),
      onTertiaryContainer: hsl(th, tp, 91),
      surface: hsl(h, n, 7),
      surfaceContainerLow: hsl(h, n, 10),
      surfaceContainer: hsl(h, n, 12.5),
      surfaceContainerHigh: hsl(h, n, 16),
      surfaceContainerHighest: hsl(h, n, 20),
      onSurface: hsl(h, n, 92),
      onSurfaceVariant: hsl(h, nv, 74),
      outline: hsl(h, nv * 0.8, 56),
      outlineVariant: hsl(h, nv * 0.8, 26),
      inverseSurface: hsl(h, n, 90),
      inverseOnSurface: hsl(h, n, 12),
      primaryHover: hsl(h, p, 82),
      primaryPressed: hsl(h, p, 70),
      success: "#5ad08a",
      warning: "#f2b84b",
      danger: "#f2867c",
      tempCool: hsl(186, 65, 60),
      tempWarm: hsl(38, 90, 60),
      tempHot: hsl(4, 85, 65),
    };
  }
  const tone = toneUnderWhiteText(h, p * 0.9, 37);
  return {
    primary: hsl(h, p * 0.9, tone),
    onPrimary: "#ffffff",
    primaryContainer: hsl(h, p, 89),
    onPrimaryContainer: hsl(h, p, 14),
    secondary: hsl(h, sec, 35),
    secondaryContainer: hsl(h, sec + 15, 88),
    onSecondaryContainer: hsl(h, sec + 10, 14),
    tertiary: hsl(th, tp * 0.75, 33),
    tertiaryContainer: hsl(th, tp * 0.9, 88),
    onTertiaryContainer: hsl(th, tp, 13),
    surface: hsl(h, n * 2.5, 98.5),
    surfaceContainerLow: hsl(h, n * 2.5, 96),
    surfaceContainer: hsl(h, n * 2, 93.5),
    surfaceContainerHigh: hsl(h, n * 2, 91),
    surfaceContainerHighest: hsl(h, n * 2, 88.5),
    onSurface: hsl(h, n, 10),
    onSurfaceVariant: hsl(h, nv, 29),
    outline: hsl(h, nv * 0.7, 44),
    outlineVariant: hsl(h, nv, 80),
    inverseSurface: hsl(h, n, 18),
    inverseOnSurface: hsl(h, n, 94),
    primaryHover: hsl(h, p * 0.9, tone - 5),
    primaryPressed: hsl(h, p * 0.9, tone - 10),
    success: "#1e8a4c",
    warning: "#a35f00",
    danger: "#c4302b",
    tempCool: hsl(186, 80, 32),
    tempWarm: hsl(32, 95, 40),
    tempHot: hsl(2, 75, 46),
  };
}

const ROLE_VARS: Record<keyof Palette, string> = {
  primary: "--md-primary",
  onPrimary: "--md-on-primary",
  primaryContainer: "--md-primary-container",
  onPrimaryContainer: "--md-on-primary-container",
  secondary: "--md-secondary",
  secondaryContainer: "--md-secondary-container",
  onSecondaryContainer: "--md-on-secondary-container",
  tertiary: "--md-tertiary",
  tertiaryContainer: "--md-tertiary-container",
  onTertiaryContainer: "--md-on-tertiary-container",
  surface: "--md-surface",
  surfaceContainerLow: "--md-surface-container-low",
  surfaceContainer: "--md-surface-container",
  surfaceContainerHigh: "--md-surface-container-high",
  surfaceContainerHighest: "--md-surface-container-highest",
  onSurface: "--md-on-surface",
  onSurfaceVariant: "--md-on-surface-variant",
  outline: "--md-outline",
  outlineVariant: "--md-outline-variant",
  inverseSurface: "--md-inverse-surface",
  inverseOnSurface: "--md-inverse-on-surface",
  primaryHover: "--md-primary-hover",
  primaryPressed: "--md-primary-pressed",
  success: "--success",
  warning: "--warning",
  danger: "--danger",
  tempCool: "--temp-cool",
  tempWarm: "--temp-warm",
  tempHot: "--temp-hot",
};

export function applyPalette(palette: Palette, root: HTMLElement = document.documentElement) {
  for (const [role, cssVar] of Object.entries(ROLE_VARS) as [keyof Palette, string][]) {
    root.style.setProperty(cssVar, palette[role]);
  }
}

export function naiveOverrides(palette: Palette): GlobalThemeOverrides {
  return {
    common: {
      fontFamily: "var(--font-sans)",
      bodyColor: palette.surface,
      cardColor: palette.surfaceContainerLow,
      modalColor: palette.surfaceContainerHigh,
      popoverColor: palette.surfaceContainerHigh,
      tableColor: palette.surfaceContainerLow,
      primaryColor: palette.primary,
      primaryColorHover: palette.primaryHover,
      primaryColorPressed: palette.primaryPressed,
      primaryColorSuppl: palette.primaryHover,
      infoColor: palette.primary,
      infoColorHover: palette.primaryHover,
      infoColorPressed: palette.primaryPressed,
      infoColorSuppl: palette.primaryHover,
      successColor: palette.success,
      warningColor: palette.warning,
      errorColor: palette.danger,
      textColorBase: palette.onSurface,
      textColor1: palette.onSurface,
      textColor2: palette.onSurface,
      textColor3: palette.onSurfaceVariant,
      placeholderColor: palette.outline,
      dividerColor: palette.outlineVariant,
      borderColor: palette.outline,
      inputColor: palette.surfaceContainerHigh,
      inputColorDisabled: palette.surfaceContainer,
      actionColor: palette.surfaceContainerHigh,
      tableHeaderColor: palette.surfaceContainer,
      hoverColor: palette.surfaceContainerHighest,
      borderRadius: "12px",
      borderRadiusSmall: "8px",
    },
    Button: {
      borderRadiusTiny: "14px",
      borderRadiusSmall: "16px",
      borderRadiusMedium: "20px",
      borderRadiusLarge: "24px",
      textColorPrimary: palette.onPrimary,
      textColorHoverPrimary: palette.onPrimary,
      textColorPressedPrimary: palette.onPrimary,
      textColorFocusPrimary: palette.onPrimary,
      fontWeight: "600",
    },
    Card: {
      borderRadius: "24px",
      color: palette.surfaceContainerLow,
      borderColor: "transparent",
    },
    Dialog: {
      borderRadius: "28px",
      color: palette.surfaceContainerHigh,
    },
    Tag: {
      borderRadius: "8px",
    },
    Alert: {
      padding: "8px",
      iconSize: "20px",
      iconMargin: "8px 8px 0 12px",
      closeMargin: "8px 10px 0 0",
      lineHeight: "1.5",
      borderRadius: "16px",
    },
  };
}
