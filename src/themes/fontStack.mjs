/** Theme API 0.3 typography grammar: the ONE definition of a bounded font-family
 * stack, shared by the app (src/themes/manifest.ts) and the package checker
 * (scripts/tine-theme.mjs), so a package the checker passes is a package the app
 * installs (GH #760). Local/system font names only: no `url()`, `@import`,
 * `@font-face`, selectors, escapes, variables or CSS-wide keywords can be said in
 * this grammar, and the CSS the host emits is rebuilt from the parsed families,
 * never copied from the manifest string.
 *
 * A stack is 1-8 comma-separated families, at most 240 characters. A family is
 *  - a generic or system keyword (`serif`, `sans-serif`, `monospace`, `system-ui`,
 *    `ui-sans-serif`, `-apple-system`, `BlinkMacSystemFont`, ...), or
 *  - a name of words made of letters, digits, `.`, `_` and `-`, separated by single
 *    spaces (at most 64 characters), bare or inside one pair of matching quotes. */

export const FONT_STACK_MAX_LENGTH = 240;
export const FONT_STACK_MAX_FAMILIES = 8;
export const FONT_FAMILY_MAX_LENGTH = 64;
/** Integer pixels; the block line box (bullet column, editor, hit-testing) is one token. */
export const BLOCK_LINE_HEIGHT_MIN = 20;
export const BLOCK_LINE_HEIGHT_MAX = 48;

const GENERIC = ["serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui", "ui-serif",
  "ui-sans-serif", "ui-monospace", "ui-rounded", "math", "emoji", "fangsong"];
const SYSTEM = ["-apple-system", "BlinkMacSystemFont"];
/** lower-case keyword -> the spelling the host emits. */
const KEYWORDS = new Map([...GENERIC, ...SYSTEM].map((name) => [name.toLowerCase(), name]));
const CSS_WIDE = new Set(["inherit", "initial", "unset", "revert", "revert-layer", "default"]);
const NAME = /^[\p{L}\p{N}\p{M}._-]+(?: [\p{L}\p{N}\p{M}._-]+)*$/u;

/** Parse a font stack. `{ ok: true, families, css }` where `families` is
 * `{ name, kind: "generic" | "system" | "named" }[]` and `css` the canonical
 * `font-family` value (named families quoted), or `{ ok: false, error }`. */
export function parseFontStack(value) {
  if (typeof value !== "string") return { ok: false, error: "must be a text font stack" };
  if (value.length === 0 || value.length > FONT_STACK_MAX_LENGTH) return { ok: false, error: `must be 1 to ${FONT_STACK_MAX_LENGTH} characters` };
  if (/[\u0000-\u001f\u007f]/.test(value)) return { ok: false, error: "must not contain control characters" };
  const parts = value.split(",");
  if (parts.length > FONT_STACK_MAX_FAMILIES) return { ok: false, error: `may name at most ${FONT_STACK_MAX_FAMILIES} families` };
  const families = [];
  for (const raw of parts) {
    const token = raw.trim();
    if (token === "") return { ok: false, error: "has an empty family" };
    const quote = token[0] === '"' || token[0] === "'" ? token[0] : "";
    let name = token;
    if (quote) {
      if (token.length < 3 || token[token.length - 1] !== quote) return { ok: false, error: `has an unterminated quoted family ${JSON.stringify(token)}` };
      name = token.slice(1, -1);
    }
    if (CSS_WIDE.has(name.toLowerCase())) return { ok: false, error: `family ${JSON.stringify(name)} is a CSS keyword` };
    // A quoted "serif" is a family that happens to be called serif, not the generic.
    const keyword = quote ? undefined : KEYWORDS.get(name.toLowerCase());
    if (keyword !== undefined) {
      families.push({ name: keyword, kind: SYSTEM.includes(keyword) ? "system" : "generic" });
      continue;
    }
    if (name.length > FONT_FAMILY_MAX_LENGTH || !NAME.test(name)) {
      return { ok: false, error: `family ${JSON.stringify(token)} must be a local font name (letters, digits, . _ - and single spaces; at most ${FONT_FAMILY_MAX_LENGTH} characters)` };
    }
    families.push({ name, kind: "named" });
  }
  const css = families.map((family) => (family.kind === "named" ? `"${family.name}"` : family.name)).join(", ");
  return { ok: true, families, css };
}

export function isBlockLineHeight(value) {
  return Number.isInteger(value) && value >= BLOCK_LINE_HEIGHT_MIN && value <= BLOCK_LINE_HEIGHT_MAX;
}
