/** Bounded literal CSS colors. Empty means the host's theme default. No CSS
 * variables, URLs, declarations, or inherited values cross this boundary. */
const n = "[+-]?(?:\\d+(?:\\.\\d+)?|\\.\\d+)";
const alpha = `${n}%?`;
const hue = `${n}(?:deg|rad|turn)?`;
const commaRgb = [n, `${n}%`].map(c => `${c},\\s*${c},\\s*${c}(?:,\\s*${alpha})?`).join("|");
export const PLUGIN_COLOR_PATTERN = `^(?:|transparent|#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})|rgba?\\((?:${commaRgb}|${n}%? +${n}%? +${n}%?(?: */ *${alpha})?)\\)|hsla?\\((?:${hue},\\s*${n}%,\\s*${n}%(?:,\\s*${alpha})?|${hue} +${n}% +${n}%(?: */ *${alpha})?)\\))$`;
const COLOR = new RegExp(PLUGIN_COLOR_PATTERN);
export function acceptsPluginColor(value) {
  return typeof value === "string" && value.length <= 100 && COLOR.test(value);
}
