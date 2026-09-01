import type { ResttyFontInput } from "restty";

export type LocalFontDetectionStatus = "ready" | "unsupported" | "denied" | "failed";

export interface LocalFontDetectionResult {
  status: LocalFontDetectionStatus;
  families: string[];
}

export interface LocalFontFaceMetadata {
  family?: string;
  fullName?: string;
  postscriptName?: string;
  style?: string;
}

interface LocalFontAccessWindow extends Window {
  queryLocalFonts?: () => Promise<LocalFontFaceMetadata[]>;
}

const MONOSPACE_PROBE = "0OQIl1MWmw0123456789[]{}()@#";
const NON_TEXT_FONT_PATTERN = /emoji|symbol|icon|dingbat|wingdings|webdings|math/i;
const MONOSPACE_TOLERANCE = 0.01;

const DEFAULT_REMOTE_FONT_INPUTS: ResttyFontInput[] = [
  {
    family: "JetBrains Mono Nerd Font",
    name: "JetBrains Mono Nerd Font Regular",
    weight: 400,
    style: "normal",
  },
  {
    family: "JetBrains Mono Nerd Font",
    name: "JetBrains Mono Nerd Font Bold",
    weight: 700,
    style: "normal",
  },
  {
    family: "JetBrains Mono Nerd Font",
    name: "JetBrains Mono Nerd Font Italic",
    weight: 400,
    style: "italic",
  },
  {
    family: "JetBrains Mono Nerd Font",
    name: "JetBrains Mono Nerd Font Bold Italic",
    weight: 700,
    style: "italic",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/JetBrainsMono/NoLigatures/Regular/JetBrainsMonoNLNerdFontMono-Regular.ttf",
    name: "JetBrains Mono Nerd Font Regular",
    weight: 400,
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/JetBrainsMono/NoLigatures/Bold/JetBrainsMonoNLNerdFontMono-Bold.ttf",
    name: "JetBrains Mono Nerd Font Bold",
    weight: 700,
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/JetBrainsMono/NoLigatures/Italic/JetBrainsMonoNLNerdFontMono-Italic.ttf",
    name: "JetBrains Mono Nerd Font Italic",
    weight: 400,
    style: "italic",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/JetBrainsMono/NoLigatures/BoldItalic/JetBrainsMonoNLNerdFontMono-BoldItalic.ttf",
    name: "JetBrains Mono Nerd Font Bold Italic",
    weight: 700,
    style: "italic",
  },
  {
    family: "Symbols Nerd Font",
    name: "Symbols Nerd Font",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/NerdFontsSymbolsOnly/SymbolsNerdFont-Regular.ttf",
    name: "Symbols Nerd Font",
  },
  {
    family: "Apple Symbols",
    name: "Apple Symbols",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/notofonts/noto-fonts@main/unhinted/ttf/NotoSansSymbols2/NotoSansSymbols2-Regular.ttf",
    name: "Noto Sans Symbols 2",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/ChiefMikeK/ttf-symbola@master/Symbola.ttf",
    name: "Symbola",
  },
  {
    family: "Noto Sans Canadian Aboriginal",
    name: "Noto Sans Canadian Aboriginal / Euphemia UCAS",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/notofonts/noto-fonts@main/unhinted/ttf/NotoSansCanadianAboriginal/NotoSansCanadianAboriginal-Regular.ttf",
    name: "Noto Sans Canadian Aboriginal",
  },
  {
    family: "Apple Color Emoji",
    name: "Apple Color Emoji",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/googlefonts/noto-emoji@main/fonts/NotoColorEmoji.ttf",
    name: "Noto Color Emoji",
  },
  {
    url: "https://cdn.jsdelivr.net/gh/hfg-gmuend/openmoji@master/font/OpenMoji-black-glyf/OpenMoji-black-glyf.ttf",
    name: "OpenMoji",
  },
];

const LOCAL_CJK_FONT_INPUTS: ResttyFontInput[] = [
  { family: "Microsoft YaHei" },
  { family: "Microsoft YaHei UI" },
  { family: "SimSun" },
  { family: "NSimSun" },
  { family: "Malgun Gothic" },
  { family: "Yu Gothic" },
  { family: "PingFang SC" },
  { family: "Hiragino Sans GB" },
  { family: "Noto Sans CJK SC" },
  { family: "Noto Sans CJK JP" },
  { family: "Noto Sans CJK KR" },
  { family: "WenQuanYi Zen Hei" },
  { family: "Source Han Sans SC" },
  {
    url: "https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf",
    name: "Noto Sans CJK SC",
  },
];

export function createTerminalFontInputs(preferredFamily: string): ResttyFontInput[] {
  const preferred = preferredFamily.trim();
  const preferredKey = preferred.toLocaleLowerCase();
  const fallbackInputs = [...DEFAULT_REMOTE_FONT_INPUTS, ...LOCAL_CJK_FONT_INPUTS].filter(
    (input) => {
      if (typeof input !== "object" || input === null || !("family" in input)) return true;
      return input.family.trim().toLocaleLowerCase() !== preferredKey;
    },
  );

  return preferred ? [{ family: preferred, local: "prefer" }, ...fallbackInputs] : fallbackInputs;
}

export function areMonospaceWidths(
  widths: readonly number[],
  tolerance = MONOSPACE_TOLERANCE,
): boolean {
  if (widths.length === 0 || widths.some((width) => !Number.isFinite(width))) return false;
  const first = widths[0];
  return widths.every((width) => Math.abs(width - first) <= tolerance);
}

export function collectMonospaceFamilies(
  faces: readonly LocalFontFaceMetadata[],
  measureFamily: (family: string) => boolean,
): string[] {
  const families = new Set<string>();

  for (const face of faces) {
    const family = face.family?.trim();
    if (!family || NON_TEXT_FONT_PATTERN.test(family) || families.has(family)) continue;
    if (measureFamily(family)) families.add(family);
  }

  return [...families].sort((a, b) => a.localeCompare(b));
}

export async function detectSystemMonospaceFonts(): Promise<LocalFontDetectionResult> {
  if (typeof window === "undefined" || !window.isSecureContext) {
    return { status: "unsupported", families: [] };
  }

  const queryLocalFonts = (window as LocalFontAccessWindow).queryLocalFonts;
  if (!queryLocalFonts) return { status: "unsupported", families: [] };

  try {
    const faces = await queryLocalFonts();
    return {
      status: "ready",
      families: collectMonospaceFamilies(faces, measureMonospaceFamily),
    };
  } catch (error) {
    const errorName = error instanceof DOMException ? error.name : "";
    return {
      status:
        errorName === "NotAllowedError" || errorName === "SecurityError" ? "denied" : "failed",
      families: [],
    };
  }
}

function measureMonospaceFamily(family: string): boolean {
  if (typeof document === "undefined") return false;

  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (!context) return false;

  const escapedFamily = family.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
  context.font = `32px "${escapedFamily}"`;
  const widths = [...MONOSPACE_PROBE].map((character) => context.measureText(character).width);
  return areMonospaceWidths(widths);
}
