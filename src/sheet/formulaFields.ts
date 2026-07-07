import { decodeFormulaExpr, formulaNameValid } from "./formula";
import type { FieldId } from "./fields";

const FORMULA_PREFIX = "tine.formula.";

export function formulasOf(props: readonly [string, string][]): ReadonlyMap<string, string> {
  const out = new Map<string, string>();
  for (const [rawKey, rawValue] of props) {
    const key = rawKey.trim();
    if (!key.toLowerCase().startsWith(FORMULA_PREFIX)) continue;
    const name = key.slice(FORMULA_PREFIX.length).trim();
    if (!formulaNameValid(name)) continue;
    out.set(name, decodeFormulaExpr(rawValue.trim()));
  }
  return out;
}

export function mergeFormulas(
  base: ReadonlyMap<string, string>,
  override: ReadonlyMap<string, string>
): ReadonlyMap<string, string> {
  const out = new Map(base);
  for (const [name, expr] of override) out.set(name, expr);
  return out;
}

export function formulaFieldId(name: string): FieldId {
  return `formula:${name}`;
}

export function formulaNameFromField(field: FieldId): string | null {
  return field.startsWith("formula:") ? field.slice("formula:".length) : null;
}
