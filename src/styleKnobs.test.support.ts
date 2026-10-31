// Test support for styleKnobs.test.ts: the knob-table validator. Only the test
// runs it (the built-in table is checked there), so it stays out of production.
import { COLOR_RE } from "./themes/manifest";
import { renderEnglish, type Msg } from "./i18n";
import { HEX_ALPHA_RE, HEX_RE, type KnobOption, type StyleKnob } from "./styleKnobs";

// --- table validation -------------------------------------------------------

const ID_RE = /^[a-z][a-z0-9-]{0,47}$/;
const CLASS_VALUE_RE = /^[a-z][a-z0-9-]{0,31}$/;
/** Values a table row may declare for a CSS variable: plain tokens, numbers,
 *  `var()`, `color-mix()`; no `;{}<>"'\@/*` and nothing that loads a resource. */
const DECLARED_VALUE_RE = /^[A-Za-z0-9 .,%()#+-]+$/;
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
