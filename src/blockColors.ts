import policy from "../fixtures/html-sanitize-policy.json";

// Logseq's built-in colors (OG 6e7afa8eb src/main/frontend/ui.cljs:59-66, `built-in-colors`),
// in Logseq's order. The one list lives in the sanitizer policy, because the sanitizer admits
// a coloured highlight's `var(--ls-highlight-color-<name>, <fallback>)` for exactly these names;
// each maps to the fixed fallback a highlight writes for viewers without Logseq's variables.
export const HIGHLIGHT_FALLBACK: Readonly<Record<string, string>> = policy.browser.highlightColors;
export const BLOCK_COLOR_NAMES: readonly string[] = Object.keys(HIGHLIGHT_FALLBACK);

// Block background colors. The swatch values match the context menu; the render tint is
// softer so text remains readable in cells.

export const BLOCK_COLOR_SWATCH: Record<string, string> = {
  yellow: "#fbe69e",
  red: "#f5a3a3",
  pink: "#f3b0d4",
  green: "#a6e3b4",
  blue: "#a8c9f0",
  purple: "#cdb4ee",
  gray: "#d3d6da",
};

export const BLOCK_COLOR_TINT: Record<string, string> = {
  yellow: "rgba(251,230,158,0.45)",
  red: "rgba(245,163,163,0.4)",
  pink: "rgba(243,176,212,0.4)",
  green: "rgba(166,227,180,0.4)",
  blue: "rgba(168,201,240,0.4)",
  purple: "rgba(205,180,238,0.4)",
  gray: "rgba(211,214,218,0.5)",
};

export function blockBackgroundColor(properties: readonly [string, string][]): string | undefined {
  const v = properties.find(([k]) => k === "background-color")?.[1];
  return v ? BLOCK_COLOR_TINT[v] ?? v : undefined;
}
