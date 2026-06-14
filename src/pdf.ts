// Shared PDF helpers (mirror the Rust asset_key / hls naming and colors).

export function assetKey(filename: string): string {
  const stem = filename.replace(/\.pdf$/i, "");
  return Array.from(stem)
    .map((c) => (/[a-z0-9]/i.test(c) ? c.toLowerCase() : "_"))
    .join("");
}

export function hlsPageName(filename: string): string {
  return `hls__${assetKey(filename)}`;
}

export const HL_COLOR_BG: Record<string, string> = {
  yellow: "rgba(255, 226, 86, 0.45)",
  green: "rgba(116, 226, 130, 0.45)",
  blue: "rgba(110, 176, 246, 0.45)",
  red: "rgba(246, 130, 130, 0.45)",
  purple: "rgba(190, 140, 246, 0.45)",
};

// Solid swatch colors for the highlight dot in the note bullet (matches OG's
// colored-dot prefix on annotation blocks).
export const HL_COLOR_SOLID: Record<string, string> = {
  yellow: "#f5c518",
  green: "#3fbf57",
  blue: "#4a9eff",
  red: "#ec5c5c",
  purple: "#a86ff0",
};

export const HL_COLORS = ["yellow", "green", "blue", "red", "purple"];
