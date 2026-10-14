/** Style knobs (GH #610 item 3): visual details that are one table row, not a
 *  Settings toggle each. `KNOBS` declares them with the Style Settings type
 *  names and fields (the subset in KNOBS-DESIGN N3); Settings > Appearance >
 *  Fine-tuning renders the table, and this module owns what reaches the page.
 *
 *  - K1: a new knob is one row in `KNOBS`; no new Settings code.
 *  - K2: only declared, validated, typed values reach CSS. The emitted text is
 *    built from the table (declared option values, an in-range number plus the
 *    declared unit, a colour matching the theme manifest grammar), never from
 *    the stored string, so a stored or imported value cannot add a declaration,
 *    `url()`, `@import` or `}`.
 *  - K3: a knob that is unset (or equal to its default) emits nothing, so the
 *    stylesheet default applies and an upgrade changes nothing.
 *  - K4: safe mode ("Disable custom CSS") also empties knob output.
 *  - K5: a device preference only (`style.knobs.v1` in the app settings, like
 *    the colour scheme); no graph write, no plugin capability.
 *
 *  Output: variables on `:root` (`--tine-<id>`) and classes on `<html>`
 *  (`tine-<id>`, `tine-<id>-<option>`), in one host-owned `<style
 *  id="tine-knobs">` placed after `#tine-theme` and before `#tine-custom-css`,
 *  so the user's custom.css still wins. This file is the only writer of that
 *  element and of the knob classes (guarded by styleKnobs.test.ts). */
import { createSignal } from "solid-js";
import { msg, renderEnglish, type Msg } from "./i18n";
import { backend } from "./backend";
import { readOwned, revisionOwner } from "./owned";
import { preferenceReadCurrent, preferenceRevision, seedPreference, writePreference } from "./preferenceWrites";
import { ensureThemeStyle } from "./themeGallery";
import { COLOR_RE } from "./themes/manifest";
import { pushToast } from "./toasts";

export const STYLE_KNOBS_STYLE_ID = "tine-knobs";
/** Device preference key: JSON `{ knobId: value }`. Unknown ids are kept. */
export const STYLE_KNOBS_KEY = "style.knobs.v1";

export type KnobOption = { label: string | Msg; value: string };
type KnobBase = { id: string; title: string | Msg; description?: string | Msg };
export type StyleKnob =
  | (KnobBase & { type: "heading"; level: 1 | 2 | 3 })
  /** On adds class `tine-<id>`. Off (the CSS default) is the only default. */
  | (KnobBase & { type: "class-toggle" })
  /** Adds class `tine-<id>-<value>`; `default` is the state with no class. */
  | (KnobBase & { type: "class-select"; default: string; options: KnobOption[] })
  | (KnobBase & { type: "variable-number-slider"; default: number; min: number; max: number; step: number; format?: string })
  | (KnobBase & { type: "variable-select"; default: string; options: KnobOption[] })
  /** Hex, with an alpha pair (`#rrggbbaa`) only when `opacity` is set. */
  | (KnobBase & { type: "variable-color"; default: string; opacity?: boolean });

export type KnobValue = boolean | number | string;
export type KnobValues = Record<string, KnobValue>;
export type KnobOutput = { css: string; classes: string[] };

/** Tine's own knobs. Later calm-cleanup rows add rows here, not toggles. A
 *  variable knob's `default` is the value the stylesheet already has (the
 *  fallback in app.css), which styleKnobs.test.ts checks. */
export const KNOBS: readonly StyleKnob[] = [
  { id: "knobs-embeds", type: "heading", level: 2, title: msg("settings-embeds") },
  {
    id: "embed-breadcrumbs", type: "class-toggle",
    title: msg("settings-embed-breadcrumbs"),
    description: msg("settings-embed-breadcrumbs-hint"),
  },
  {
    id: "embed-shading",
    type: "class-select",
    title: msg("settings-embedded-blocks"),
    description: msg("settings-shade-embedded-blocks-like-logseq-or-show"),
    default: "shade",
    options: [
      { label: msg("settings-shaded-box"), value: "shade" },
      { label: msg("settings-accent-line-and-dot-only"), value: "accent" },
    ],
  },
  { id: "knobs-links", type: "heading", level: 2, title: msg("settings-links") },
  {
    id: "external-link-underline",
    type: "variable-select",
    title: msg("settings-links-to-the-web-or-a-file"),
    description: msg("settings-the-line-under-a-link-that-leaves"),
    default: "1px solid",
    options: [
      { label: msg("settings-solid-underline"), value: "1px solid" },
      { label: msg("settings-dotted-underline"), value: "1px dotted" },
      { label: msg("settings-no-underline"), value: "none" },
    ],
  },
  {
    id: "missing-page-color",
    type: "variable-select",
    title: msg("settings-links-to-pages-that-do-not-exist"),
    description: msg("settings-how-strongly-the-dotted-link-to-a"),
    default: "color-mix(in srgb, var(--link-color) 55%, var(--text-secondary))",
    options: [
      { label: msg("settings-dimmed-link-color"), value: "color-mix(in srgb, var(--link-color) 55%, var(--text-secondary))" },
      { label: msg("settings-stronger"), value: "color-mix(in srgb, var(--link-color) 80%, var(--text-secondary))" },
      { label: msg("settings-same-as-other-links"), value: "var(--link-color)" },
    ],
  },
];

// --- table validation -------------------------------------------------------

const ID_RE = /^[a-z][a-z0-9-]{0,47}$/;
const CLASS_VALUE_RE = /^[a-z][a-z0-9-]{0,31}$/;
/** Values a table row may declare for a CSS variable: plain tokens, numbers,
 *  `var()`, `color-mix()`; no `;{}<>"'\@/*` and nothing that loads a resource. */
const DECLARED_VALUE_RE = /^[A-Za-z0-9 .,%()#+-]+$/;
const HEX_RE = /^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{6})$/;
const HEX_ALPHA_RE = /^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$/;
const UNITS = ["", "px", "em", "rem", "%", "ch", "vw", "vh", "deg", "ms", "s"];

function sourceText(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (value && typeof value === "object" && "kind" in value && value.kind === "msg" && "id" in value && typeof value.id === "string") return renderEnglish(value as Msg);
  return null;
}

function declaredValueProblem(value: unknown): string | null {
  if (typeof value !== "string" || value.length === 0 || value.length > 120) return "must be 1-120 characters";
  if (!DECLARED_VALUE_RE.test(value)) return "may use only letters, digits, space and . , % ( ) # + -";
  if (/url\(|image\(|image-set\(|expression\(|src\(/i.test(value)) return "may not load a resource";
  let depth = 0;
  for (const ch of value) {
    if (ch === "(") depth++;
    if (ch === ")" && --depth < 0) return "has unbalanced parentheses";
  }
  return depth === 0 ? null : "has unbalanced parentheses";
}

function optionProblems(knob: StyleKnob & { options: KnobOption[]; default: string }, where: string, classes: boolean): string[] {
  const problems: string[] = [];
  if (!Array.isArray(knob.options) || knob.options.length < 2 || knob.options.length > 12) {
    return [`${where}: needs 2-12 options`];
  }
  const seen = new Set<string>();
  for (const option of knob.options) {
    const label = sourceText(option?.label);
    if (!label || !label.trim() || label.length > 80) problems.push(`${where}: option label must be 1-80 characters`);
    const bad = classes ? (CLASS_VALUE_RE.test(option?.value ?? "") ? null : "must match [a-z][a-z0-9-]{0,31}") : declaredValueProblem(option?.value);
    if (bad) problems.push(`${where}: option value ${JSON.stringify(option?.value)} ${bad}`);
    if (seen.has(option?.value)) problems.push(`${where}: duplicate option value ${JSON.stringify(option?.value)}`);
    seen.add(option?.value);
  }
  if (!seen.has(knob.default)) problems.push(`${where}: default ${JSON.stringify(knob.default)} is not one of the options`);
  return problems;
}

/** Everything wrong with a knob table (empty = valid): unique ids, class and
 *  variable names that cannot collide, defaults that are valid values. */
export function validateKnobTable(table: readonly StyleKnob[]): string[] {
  const problems: string[] = [];
  const ids = new Set<string>();
  const classes = new Set<string>();
  const claimClass = (name: string, where: string) => {
    if (classes.has(name)) problems.push(`${where}: class ${name} is claimed twice`);
    classes.add(name);
  };
  for (const knob of table) {
    const where = `knob ${JSON.stringify(knob?.id)}`;
    if (!knob || typeof knob.id !== "string" || !ID_RE.test(knob.id)) { problems.push(`${where}: id must match [a-z][a-z0-9-]{0,47}`); continue; }
    if (ids.has(knob.id)) problems.push(`${where}: duplicate id`);
    ids.add(knob.id);
    const title = sourceText(knob.title);
    const description = sourceText(knob.description);
    if (!title || !title.trim() || title.length > 80) problems.push(`${where}: title must be 1-80 characters`);
    if (knob.description !== undefined && (description === null || description.length > 400)) problems.push(`${where}: description must be at most 400 characters`);
    switch (knob.type) {
      case "heading":
        if (![1, 2, 3].includes(knob.level)) problems.push(`${where}: level must be 1-3`);
        break;
      case "class-toggle":
        claimClass(`tine-${knob.id}`, where);
        break;
      case "class-select":
        problems.push(...optionProblems(knob, where, true));
        for (const option of knob.options ?? []) claimClass(`tine-${knob.id}-${option?.value}`, where);
        break;
      case "variable-select":
        problems.push(...optionProblems(knob, where, false));
        break;
      case "variable-number-slider": {
        const { min, max, step } = knob;
        const finite = [knob.default, min, max, step].every((n) => typeof n === "number" && Number.isFinite(n));
        if (!finite || !(min < max) || !(step > 0) || knob.default < min || knob.default > max) problems.push(`${where}: needs finite min < max, step > 0 and a default within [min, max]`);
        if (!UNITS.includes(knob.format ?? "")) problems.push(`${where}: unit ${JSON.stringify(knob.format)} is not allowed`);
        break;
      }
      case "variable-color":
        if (!(typeof knob.default === "string" && (knob.opacity ? HEX_ALPHA_RE : HEX_RE).test(knob.default) && COLOR_RE.test(knob.default))) problems.push(`${where}: default must be a hex colour`);
        break;
      default:
        problems.push(`${where}: unknown type ${JSON.stringify((knob as { type?: unknown }).type)}`);
    }
  }
  return problems;
}

// --- values -----------------------------------------------------------------

/** The value a knob would emit for `value`, or `null` when `value` is not
 *  valid for the knob (wrong type, outside the declared range or list). */
export function validKnobValue(knob: StyleKnob, value: unknown): KnobValue | null {
  switch (knob.type) {
    case "heading":
      return null;
    case "class-toggle":
      return typeof value === "boolean" ? value : null;
    case "class-select":
    case "variable-select":
      return typeof value === "string" && knob.options.some((option) => option.value === value) ? value : null;
    case "variable-number-slider":
      return typeof value === "number" && Number.isFinite(value) && value >= knob.min && value <= knob.max ? value : null;
    case "variable-color":
      return typeof value === "string" && value.length <= 100 && (knob.opacity ? HEX_ALPHA_RE : HEX_RE).test(value) && COLOR_RE.test(value) ? value.toLowerCase() : null;
  }
}

/** Whether `value` is a real choice: valid and different from the default. */
function isActive(knob: StyleKnob, value: KnobValue | null): value is KnobValue {
  if (value === null) return false;
  if (knob.type === "class-toggle") return value === true;
  if (knob.type === "heading") return false;
  return knob.type === "variable-color" ? value !== knob.default.toLowerCase() : value !== knob.default;
}

/** Whether the stored values hold a real choice for `knob` (drives Reset). */
export function knobIsSet(knob: StyleKnob, values: KnobValues): boolean {
  return isActive(knob, validKnobValue(knob, values[knob.id]));
}

/** The CSS variables and html classes for the stored values. Pure: each emitted
 *  piece comes from the table, an in-range number or a validated colour.
 *  O(table size). */
export function knobOutput(table: readonly StyleKnob[], values: KnobValues): KnobOutput {
  const declarations: string[] = [];
  const classes: string[] = [];
  for (const knob of table) {
    const value = validKnobValue(knob, values[knob.id]);
    if (!isActive(knob, value)) continue;
    switch (knob.type) {
      case "class-toggle":
        classes.push(`tine-${knob.id}`);
        break;
      case "class-select":
        classes.push(`tine-${knob.id}-${value}`);
        break;
      case "variable-select":
        declarations.push(`--tine-${knob.id}: ${value};`);
        break;
      case "variable-number-slider":
        declarations.push(`--tine-${knob.id}: ${value}${knob.format ?? ""};`);
        break;
      case "variable-color":
        declarations.push(`--tine-${knob.id}: ${value};`);
        break;
    }
  }
  return { css: declarations.length ? `:root { ${declarations.join(" ")} }` : "", classes };
}

const MAX_STORED_KNOBS = 256;
/** Parse stored text without letting a corrupt string throw; the caller maps
 *  "not JSON" to "nothing stored" (a device preference holds no data to lose). */
function parseJson(text: string): { ok: true; value: unknown } | { ok: false } {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch {
    return { ok: false };
  }
}

/** Parse the stored JSON; keep every scalar entry (unknown ids included) and
 *  drop everything else. Never throws; unusable text means no stored choices. */
export function decodeKnobValues(text: string): KnobValues {
  const out: KnobValues = {};
  const parsed = parseJson(text);
  if (!parsed.ok || !parsed.value || typeof parsed.value !== "object" || Array.isArray(parsed.value)) return out;
  for (const [key, value] of Object.entries(parsed.value).slice(0, MAX_STORED_KNOBS)) {
    if (key.length > 80) continue;
    if (typeof value === "boolean" || typeof value === "number" || (typeof value === "string" && value.length <= 200)) out[key] = value;
  }
  return out;
}

// --- runtime ----------------------------------------------------------------

const [values, setValues] = createSignal<KnobValues>({});
/** The stored knob values (unvalidated; `knobOutput` validates on use). */
export const styleKnobValues = values;

let safeMode = false;
let appliedClasses: string[] = [];

function ensureKnobsStyle(): HTMLStyleElement | null {
  if (typeof document === "undefined") return null;
  const theme = ensureThemeStyle();
  let el = document.getElementById(STYLE_KNOBS_STYLE_ID) as HTMLStyleElement | null;
  if (el && el.tagName !== "STYLE") {
    el.remove();
    el = null;
  }
  if (!el) {
    el = document.createElement("style");
    el.id = STYLE_KNOBS_STYLE_ID;
  }
  // Right after #tine-theme: custom.css (after the theme) keeps the last word.
  if (theme?.parentNode === document.head) {
    if (theme.nextSibling !== el) document.head.insertBefore(el, theme.nextSibling);
  } else if (el.parentNode !== document.head) {
    document.head.appendChild(el);
  }
  return el;
}

/** Repaint `#tine-knobs` and the `<html>` classes from the stored values; safe
 *  mode paints nothing. */
export function paintKnobs(): void {
  const el = ensureKnobsStyle();
  if (!el) return;
  const output = safeMode ? { css: "", classes: [] } : knobOutput(KNOBS, values());
  if (el.textContent !== output.css) el.textContent = output.css;
  const root = document.documentElement;
  for (const name of appliedClasses) if (!output.classes.includes(name)) root.classList.remove(name);
  for (const name of output.classes) root.classList.add(name);
  appliedClasses = output.classes;
}

/** K4: the "Disable custom CSS" safe mode also silences the knobs. */
export function setKnobsSafeMode(disabled: boolean): void {
  safeMode = disabled;
  paintKnobs();
}

function applyKnobValues(next: KnobValues): void {
  setValues(next);
  paintKnobs();
}

/** Choose a value for a knob (or `undefined`: reset it). An invalid value or
 *  the default is stored as "unset". Applies now, persists on this device;
 *  a failed latest write rolls back and toasts. */
export function setKnob(knob: StyleKnob, value: KnobValue | undefined): void {
  const next: KnobValues = { ...values() };
  const checked = value === undefined ? null : validKnobValue(knob, value);
  if (isActive(knob, checked)) next[knob.id] = checked;
  else delete next[knob.id];
  writePreference(values, applyKnobValues, next,
    (written) => backend().setAppString(STYLE_KNOBS_KEY, JSON.stringify(written)), "style settings");
}

/** Load the stored values at startup and paint. A read failure toasts and
 *  leaves the defaults; a choice made meanwhile wins. */
export async function initStyleKnobs(): Promise<void> {
  ensureKnobsStyle();
  const revision = preferenceRevision(values);
  const owner = revisionOwner(values, revision, () => preferenceReadCurrent(values, revision));
  let stored: KnobValues = {};
  try {
    const result = await readOwned(owner, backend().getAppString(STYLE_KNOBS_KEY, ""));
    if (result.kind === "stale") return;
    stored = decodeKnobValues(result.value);
  } catch {
    pushToast(msg("settings-could-not-load-style-settings"), "error");
  }
  if (owner()) {
    applyKnobValues(stored);
    seedPreference(values);
  }
}
