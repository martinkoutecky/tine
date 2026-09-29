export const COLORS = ["yellow", "green", "blue", "red", "purple"];
export const COLOR_RGB: Record<string, string> = {
  yellow: "255, 226, 86",
  green: "116, 226, 130",
  blue: "110, 176, 246",
  red: "246, 130, 130",
  purple: "190, 140, 246",
};
export const COLOR_RGBA: Record<string, string> = Object.fromEntries(
  Object.entries(COLOR_RGB).map(([key, value]) => [key, `rgba(${value}, 0.4)`]),
);
export const PDF_THEMES = ["light", "warm", "dark"] as const;
export type PdfTheme = (typeof PDF_THEMES)[number];
