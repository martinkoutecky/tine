import { parseIsoDateLike, type IsoDateParts } from "../typed";

export type DurationUnit = "s" | "m" | "h" | "d" | "w" | "M" | "y";

export type FormulaValue =
  | { kind: "text"; value: string }
  | { kind: "number"; value: number }
  | { kind: "boolean"; value: boolean }
  | { kind: "date"; value: IsoDateParts; source: string }
  | { kind: "duration"; n: number; unit: DurationUnit }
  | { kind: "list"; values: FormulaValue[] }
  | { kind: "null" }
  | { kind: "error"; message: string };

export type FormulaDateValue = Extract<FormulaValue, { kind: "date" }>;
export type FormulaDurationValue = Extract<FormulaValue, { kind: "duration" }>;
export type FormulaErrorValue = Extract<FormulaValue, { kind: "error" }>;

export function textValue(value: string): FormulaValue {
  return { kind: "text", value };
}

export function numberValue(value: number): FormulaValue {
  return { kind: "number", value };
}

export function booleanValue(value: boolean): FormulaValue {
  return { kind: "boolean", value };
}

export function nullValue(): FormulaValue {
  return { kind: "null" };
}

export function errorValue(message: string): FormulaErrorValue {
  return { kind: "error", message };
}

export function isErrorValue(value: FormulaValue): value is FormulaErrorValue {
  return value.kind === "error";
}

export function parseDateValue(source: string): FormulaDateValue | null {
  const value = parseIsoDateLike(source);
  return value ? { kind: "date", value, source } : null;
}

export function makeDateValue(y: number, m: number, d: number, hour = 0, minute = 0, includeTime = false): FormulaDateValue {
  const date = `${String(y).padStart(4, "0")}-${String(m + 1).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
  const time = includeTime || hour !== 0 || minute !== 0 ? ` ${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}` : "";
  const source = `${date}${time}`;
  const parsed = parseDateValue(source);
  if (!parsed) return { kind: "date", value: { y, m, d, time: time ? time.slice(1) : null }, source };
  return parsed;
}

export function dateValueFromDate(date: Date, includeTime: boolean): FormulaDateValue {
  return makeDateValue(
    date.getUTCFullYear(),
    date.getUTCMonth(),
    date.getUTCDate(),
    includeTime ? date.getUTCHours() : 0,
    includeTime ? date.getUTCMinutes() : 0,
    includeTime,
  );
}

export function dateToUtcDate(value: FormulaDateValue): Date {
  const time = value.value.time;
  const hour = time ? Number(time.slice(0, 2)) : 0;
  const minute = time ? Number(time.slice(3, 5)) : 0;
  return new Date(Date.UTC(value.value.y, value.value.m, value.value.d, hour, minute));
}

export function parseDurationValue(value: string): FormulaDurationValue | null {
  const m = /^(\d+)([smhdwMy])$/.exec(value);
  if (!m) return null;
  return { kind: "duration", n: Number(m[1]), unit: m[2] as DurationUnit };
}
