/** Theme API 0.3 typography (GH #760): what a package may say about type, and the
 * CSS the host builds from it. The grammar lives in `fontStack.mjs` (shared with
 * `scripts/tine-theme.mjs`); this file owns the CSS Tine generates. Packages stay
 * inert: only the parsed families and an in-range integer reach the stylesheet. */
import {
  BLOCK_LINE_HEIGHT_MAX,
  BLOCK_LINE_HEIGHT_MIN,
  isBlockLineHeight,
  parseFontStack,
  type FontFamily,
} from "./fontStack.mjs";

export { BLOCK_LINE_HEIGHT_MAX, BLOCK_LINE_HEIGHT_MIN };

export interface ThemeTypography {
  /** Body text of rendered blocks and the interface (`--tine-content-font`). */
  bodyFont?: string;
  /** Code, inline code and the code editor (`--tine-mono-font`). */
  monoFont?: string;
  /** Page titles and heading blocks (rules below). */
  titleFont?: string;
  /** Even integer px height (20 to 48) of a block's first line (`--ls-block-line-height`). */
  blockLineHeight?: number;
}

export const TYPOGRAPHY_FIELDS = ["bodyFont", "monoFont", "titleFont", "blockLineHeight"] as const;
const FONT_FIELDS = ["bodyFont", "monoFont", "titleFont"] as const;

/** Throws `Error(message)` naming the first thing wrong; the manifest parser
 * rewraps it as a ThemeManifestError. Unknown keys are the caller's check. */
export function checkTypography(field: (typeof TYPOGRAPHY_FIELDS)[number], value: unknown): void {
  if (field === "blockLineHeight") {
    if (!isBlockLineHeight(value)) {
      throw new Error(`typography.blockLineHeight must be an even integer from ${BLOCK_LINE_HEIGHT_MIN} to ${BLOCK_LINE_HEIGHT_MAX}`);
    }
    return;
  }
  const parsed = parseFontStack(value);
  if (!parsed.ok) throw new Error(`typography.${field} ${parsed.error}`);
}

export function hasTypography(typography: ThemeTypography | undefined): boolean {
  return !!typography && TYPOGRAPHY_FIELDS.some((field) => typography[field] !== undefined);
}

/** A stack for editable text (textareas, inputs, emoji runs): the emoji face goes
 * before the final generic family, as the editorial preset does, so a generic can
 * never resolve to the crashing system color-emoji font first (GH #458). */
function editableStack(families: readonly FontFamily[]): string {
  const css = (family: FontFamily) => (family.kind === "named" ? `"${family.name}"` : family.name);
  const last = families[families.length - 1];
  if (last && last.kind === "generic") {
    return [...families.slice(0, -1).map(css), "var(--tine-editable-emoji-font)", css(last)].join(", ");
  }
  return [...families.map(css), "var(--tine-editable-emoji-font)"].join(", ");
}

const HEADING_EDITORS = [1, 2, 3, 4, 5, 6].map((n) => `.block-editor.h${n}`).join(",\n");
/** Page titles, heading blocks and the editorial journal title (which ties the
 * editorial preset's own face on specificity and wins by coming later). */
const TITLE_SELECTORS = `.page-title,\n.page-title-input,\n.heading-text,\nhtml[data-theme-journal-header="editorial"] .journal-title`;

/** The host-owned stylesheet text for a package's typography ("" when it has
 * none). It sets the public `--tine-*` font tokens on `:root`, in the theme
 * element that precedes custom.css, so a user's own token still wins. Never
 * throws: a field that does not parse is skipped (the manifest parser already
 * refused it; this is the second fence). */
export function typographyCss(typography: ThemeTypography | undefined): string {
  if (!typography) return "";
  const root: string[] = [];
  const rules: string[] = [];
  const body = parseFontStack(typography.bodyFont);
  if (body.ok) {
    root.push(`--tine-content-font: ${body.css};`, `--tine-editable-font: ${editableStack(body.families)};`);
  }
  const mono = parseFontStack(typography.monoFont);
  if (mono.ok) root.push(`--tine-mono-font: ${mono.css};`);
  const title = parseFontStack(typography.titleFont);
  if (title.ok) {
    rules.push(`${TITLE_SELECTORS} {\n  font-family: ${title.css};\n}`);
    rules.push(`${HEADING_EDITORS} {\n  --tine-editable-font: ${editableStack(title.families)};\n}`);
  }
  if (isBlockLineHeight(typography.blockLineHeight)) root.push(`--tine-typography-line-height: ${typography.blockLineHeight}px;`);
  if (root.length > 0) rules.unshift(`:root {\n  ${root.join("\n  ")}\n}`);
  return rules.join("\n\n");
}

/** The first family of each declared stack that names an installable font (not a
 * generic or system keyword), for the "not installed" hint. */
export function leadingFamilies(typography: ThemeTypography | undefined): string[] {
  if (!typography) return [];
  const names = new Set<string>();
  for (const field of FONT_FIELDS) {
    const parsed = parseFontStack(typography[field]);
    const first = parsed.ok ? parsed.families[0] : undefined;
    if (first && first.kind === "named") names.add(first.name);
  }
  return [...names];
}
