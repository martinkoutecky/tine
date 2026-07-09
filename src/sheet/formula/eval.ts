import type { Ast, BinaryOp } from "./parser";
import {
  booleanValue,
  dateToUtcDate,
  dateValueFromDate,
  errorValue,
  isErrorValue,
  makeDateValue,
  nullValue,
  numberValue,
  parseDurationValue,
  textValue,
  type FormulaDateValue,
  type FormulaDurationValue,
  type FormulaErrorValue,
  type FormulaValue,
} from "./value";

export interface FormulaEvalContext {
  field(name: string): FormulaValue;
  formulaAst(name: string): Ast | null;
  now: Date;
}

type MemberHandler = (target: FormulaValue, args: readonly FormulaValue[] | null, ctx: FormulaEvalContext) => FormulaValue;
type MemberTable = Partial<Record<FormulaValue["kind"], Record<string, MemberHandler>>>;

const DAY_MS = 24 * 60 * 60 * 1000;

function typeName(value: FormulaValue): string {
  return value.kind;
}

function fixedArgs(name: string, args: readonly FormulaValue[] | null, count: number): readonly FormulaValue[] | FormulaErrorValue {
  if (args == null) return errorValue(`${name} must be called with ()`);
  return args.length === count ? args : errorValue(`${name} expects ${count} argument${count === 1 ? "" : "s"}`);
}

function isArgList(value: readonly FormulaValue[] | FormulaErrorValue): value is readonly FormulaValue[] {
  return Array.isArray(value);
}

function propertyOrZeroArity(name: string, args: readonly FormulaValue[] | null): FormulaErrorValue | null {
  if (args == null || args.length === 0) return null;
  return errorValue(`${name} expects 0 arguments`);
}

function textArg(name: string, args: readonly FormulaValue[], index: number): string | FormulaErrorValue {
  const value = args[index];
  return value.kind === "text" ? value.value : errorValue(`${name} argument ${index + 1} expects text`);
}

function numberArg(name: string, args: readonly FormulaValue[], index: number): number | FormulaErrorValue {
  const value = args[index];
  return value.kind === "number" ? value.value : errorValue(`${name} argument ${index + 1} expects number`);
}

function firstErrorDeep(value: FormulaValue): FormulaErrorValue | null {
  if (isErrorValue(value)) return value;
  if (value.kind === "list") {
    for (const item of value.values) {
      const found = firstErrorDeep(item);
      if (found) return found;
    }
  }
  return null;
}

function durationFromValue(value: FormulaValue): FormulaDurationValue | null {
  if (value.kind === "duration") return value;
  if (value.kind === "text") return parseDurationValue(value.value);
  return null;
}

function daysInMonth(y: number, m: number): number {
  return new Date(Date.UTC(y, m + 1, 0)).getUTCDate();
}

function addMonths(value: FormulaDateValue, delta: number): FormulaValue {
  const source = dateToUtcDate(value);
  const monthIndex = source.getUTCFullYear() * 12 + source.getUTCMonth() + delta;
  const y = Math.floor(monthIndex / 12);
  const m = ((monthIndex % 12) + 12) % 12;
  const d = Math.min(source.getUTCDate(), daysInMonth(y, m));
  // Calendar month/year math clamps the day to the target month's end instead
  // of allowing JS Date overflow (Jan 31 + 1M => Feb 28/29, not March).
  return makeDateValue(y, m, d, source.getUTCHours(), source.getUTCMinutes(), value.value.time != null);
}

function addDuration(value: FormulaDateValue, duration: FormulaDurationValue, sign: 1 | -1): FormulaValue {
  const n = duration.n * sign;
  if (duration.unit === "M") return addMonths(value, n);
  if (duration.unit === "y") return addMonths(value, n * 12);

  const unitMs: Record<Exclude<FormulaDurationValue["unit"], "M" | "y">, number> = {
    s: 1000,
    m: 60 * 1000,
    h: 60 * 60 * 1000,
    d: DAY_MS,
    w: 7 * DAY_MS,
  };
  const date = new Date(dateToUtcDate(value).getTime() + n * unitMs[duration.unit]);
  const includeTime = value.value.time != null || duration.unit === "s" || duration.unit === "m" || duration.unit === "h";
  return dateValueFromDate(date, includeTime);
}

function compareValues(op: BinaryOp, left: FormulaValue, right: FormulaValue): FormulaValue {
  if (left.kind !== right.kind) return errorValue(`Cannot compare ${typeName(left)} and ${typeName(right)}`);

  let cmp: number | null = null;
  if (left.kind === "number" && right.kind === "number") cmp = left.value === right.value ? 0 : left.value < right.value ? -1 : 1;
  else if (left.kind === "text" && right.kind === "text") cmp = left.value === right.value ? 0 : left.value < right.value ? -1 : 1;
  else if (left.kind === "boolean" && right.kind === "boolean") cmp = left.value === right.value ? 0 : left.value ? 1 : -1;
  else if (left.kind === "date" && right.kind === "date") {
    const l = dateToUtcDate(left).getTime();
    const r = dateToUtcDate(right).getTime();
    cmp = l === r ? 0 : l < r ? -1 : 1;
  }

  if (cmp == null) return errorValue(`Operator ${op} is not defined for ${typeName(left)}`);
  switch (op) {
    case "<":
      return booleanValue(cmp < 0);
    case "<=":
      return booleanValue(cmp <= 0);
    case ">":
      return booleanValue(cmp > 0);
    case ">=":
      return booleanValue(cmp >= 0);
    default:
      return errorValue(`Operator ${op} is not a comparison`);
  }
}

function deepEqual(left: FormulaValue, right: FormulaValue): boolean {
  if (left.kind !== right.kind) return false;
  switch (left.kind) {
    case "text":
    case "number":
    case "boolean":
      return left.value === (right as typeof left).value;
    case "date":
      return right.kind === "date" && dateToUtcDate(left).getTime() === dateToUtcDate(right).getTime();
    case "duration":
      return right.kind === "duration" && left.n === right.n && left.unit === right.unit;
    case "list":
      return right.kind === "list" && left.values.length === right.values.length && left.values.every((value, i) => deepEqual(value, right.values[i]));
    case "null":
      return true;
    case "error":
      return false;
  }
}

function displayValue(value: FormulaValue): string | FormulaErrorValue {
  if (isErrorValue(value)) return value;
  switch (value.kind) {
    case "text":
      return value.value;
    case "number":
      return String(value.value);
    case "boolean":
      return value.value ? "true" : "false";
    case "date":
      return value.source;
    case "duration":
      return `${value.n}${value.unit}`;
    case "list": {
      const out: string[] = [];
      for (const item of value.values) {
        const displayed = displayValue(item);
        if (typeof displayed !== "string") return displayed;
        out.push(displayed);
      }
      return out.join(",");
    }
    case "null":
      return "";
  }
}

function isEmpty(value: FormulaValue): FormulaValue {
  if (isErrorValue(value)) return value;
  if (value.kind === "null") return booleanValue(true);
  if (value.kind === "text") return booleanValue(value.value.length === 0);
  if (value.kind === "list") return booleanValue(value.values.length === 0);
  return booleanValue(false);
}

function formatDate(value: FormulaDateValue, fmt: string): string {
  const date = dateToUtcDate(value);
  const tokens: Record<string, string> = {
    YYYY: String(date.getUTCFullYear()).padStart(4, "0"),
    MM: String(date.getUTCMonth() + 1).padStart(2, "0"),
    DD: String(date.getUTCDate()).padStart(2, "0"),
    HH: String(date.getUTCHours()).padStart(2, "0"),
    mm: String(date.getUTCMinutes()).padStart(2, "0"),
  };
  // `.format` intentionally recognizes only these ADR tokens; all other
  // characters pass through literally.
  return fmt.replace(/YYYY|MM|DD|HH|mm/g, (token) => tokens[token]);
}

function relativeDate(value: FormulaDateValue, ctx: FormulaEvalContext): FormulaValue {
  const target = Date.UTC(value.value.y, value.value.m, value.value.d);
  const today = Date.UTC(ctx.now.getUTCFullYear(), ctx.now.getUTCMonth(), ctx.now.getUTCDate());
  const days = Math.round((target - today) / DAY_MS);
  if (days === 0) return textValue("today");
  return textValue(days > 0 ? `in ${days}d` : `${Math.abs(days)}d ago`);
}

const MEMBER_TABLE: MemberTable = {
  text: {
    contains(target, args) {
      const values = fixedArgs("contains", args, 1);
      if (!isArgList(values)) return values;
      const needle = textArg("contains", values, 0);
      return typeof needle === "string" ? booleanValue(target.kind === "text" && target.value.includes(needle)) : needle;
    },
    lower(target, args) {
      const values = fixedArgs("lower", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "text" ? textValue(target.value.toLowerCase()) : errorValue("lower expects text");
    },
    trim(target, args) {
      const values = fixedArgs("trim", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "text" ? textValue(target.value.trim()) : errorValue("trim expects text");
    },
    replace(target, args) {
      const values = fixedArgs("replace", args, 2);
      if (!isArgList(values)) return values;
      const search = textArg("replace", values, 0);
      if (typeof search !== "string") return search;
      const replacement = textArg("replace", values, 1);
      if (typeof replacement !== "string") return replacement;
      return target.kind === "text" ? textValue(target.value.replace(search, replacement)) : errorValue("replace expects text");
    },
    length(target, args) {
      const arityError = propertyOrZeroArity("length", args);
      return arityError ?? (target.kind === "text" ? numberValue(target.value.length) : errorValue("length expects text"));
    },
  },
  number: {
    round(target, args) {
      const values = fixedArgs("round", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "number" ? numberValue(Math.round(target.value)) : errorValue("round expects number");
    },
    floor(target, args) {
      const values = fixedArgs("floor", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "number" ? numberValue(Math.floor(target.value)) : errorValue("floor expects number");
    },
    ceil(target, args) {
      const values = fixedArgs("ceil", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "number" ? numberValue(Math.ceil(target.value)) : errorValue("ceil expects number");
    },
    abs(target, args) {
      const values = fixedArgs("abs", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "number" ? numberValue(Math.abs(target.value)) : errorValue("abs expects number");
    },
    toFixed(target, args) {
      const values = fixedArgs("toFixed", args, 1);
      if (!isArgList(values)) return values;
      const places = numberArg("toFixed", values, 0);
      if (typeof places !== "number") return places;
      if (!Number.isInteger(places) || places < 0 || places > 100) return errorValue("toFixed argument 1 expects an integer from 0 to 100");
      return target.kind === "number" ? textValue(target.value.toFixed(places)) : errorValue("toFixed expects number");
    },
  },
  date: {
    format(target, args) {
      const values = fixedArgs("format", args, 1);
      if (!isArgList(values)) return values;
      const fmt = textArg("format", values, 0);
      if (typeof fmt !== "string") return fmt;
      return target.kind === "date" ? textValue(formatDate(target, fmt)) : errorValue("format expects date");
    },
    relative(target, args, ctx) {
      const values = fixedArgs("relative", args, 0);
      if (!isArgList(values)) return values;
      return target.kind === "date" ? relativeDate(target, ctx) : errorValue("relative expects date");
    },
    year(target, args) {
      const arityError = propertyOrZeroArity("year", args);
      return arityError ?? (target.kind === "date" ? numberValue(target.value.y) : errorValue("year expects date"));
    },
    month(target, args) {
      const arityError = propertyOrZeroArity("month", args);
      return arityError ?? (target.kind === "date" ? numberValue(target.value.m + 1) : errorValue("month expects date"));
    },
    day(target, args) {
      const arityError = propertyOrZeroArity("day", args);
      return arityError ?? (target.kind === "date" ? numberValue(target.value.d) : errorValue("day expects date"));
    },
  },
  list: {
    length(target, args) {
      const arityError = propertyOrZeroArity("length", args);
      return arityError ?? (target.kind === "list" ? numberValue(target.values.length) : errorValue("length expects list"));
    },
    join(target, args) {
      const values = fixedArgs("join", args, 1);
      if (!isArgList(values)) return values;
      const sep = textArg("join", values, 0);
      if (typeof sep !== "string") return sep;
      if (target.kind !== "list") return errorValue("join expects list");
      const out: string[] = [];
      for (const item of target.values) {
        const displayed = displayValue(item);
        if (typeof displayed !== "string") return displayed;
        out.push(displayed);
      }
      return textValue(out.join(sep));
    },
    contains(target, args) {
      const values = fixedArgs("contains", args, 1);
      if (!isArgList(values)) return values;
      if (target.kind !== "list") return errorValue("contains expects list");
      const needleError = firstErrorDeep(values[0]);
      if (needleError) return needleError;
      for (const item of target.values) {
        const itemError = firstErrorDeep(item);
        if (itemError) return itemError;
        if (deepEqual(item, values[0])) return booleanValue(true);
      }
      return booleanValue(false);
    },
  },
};

function evalArgs(args: readonly Ast[], ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue[] | FormulaErrorValue {
  const out: FormulaValue[] = [];
  for (const arg of args) {
    const value = evalAst(arg, ctx, visited);
    if (isErrorValue(value)) return value;
    out.push(value);
  }
  return out;
}

function evalField(name: string, ctx: FormulaEvalContext): FormulaValue {
  try {
    return ctx.field(name);
  } catch (err) {
    return errorValue(`Field ${name} failed: ${err instanceof Error ? err.message : String(err)}`);
  }
}

function evalFormulaRef(name: string, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  const prior = visited.indexOf(name);
  if (prior >= 0) return errorValue(`Formula cycle: ${[...visited.slice(prior), name].join(" -> ")}`);

  let ast: Ast | null;
  try {
    ast = ctx.formulaAst(name);
  } catch (err) {
    return errorValue(`Formula ${name} failed: ${err instanceof Error ? err.message : String(err)}`);
  }
  if (!ast) return errorValue(`Unknown formula ${name}`);
  return evalAst(ast, ctx, [...visited, name]);
}

function evalUnary(op: "!" | "-", expr: Ast, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  const value = evalAst(expr, ctx, visited);
  if (isErrorValue(value)) return value;
  if (op === "!") return value.kind === "boolean" ? booleanValue(!value.value) : errorValue("! expects boolean");
  return value.kind === "number" ? numberValue(-value.value) : errorValue("Unary - expects number");
}

function evalLogical(op: "&&" | "||", leftAst: Ast, rightAst: Ast, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  const left = evalAst(leftAst, ctx, visited);
  if (isErrorValue(left)) return left;
  if (left.kind !== "boolean") return errorValue(`${op} expects boolean operands`);
  if (op === "&&" && !left.value) return booleanValue(false);
  if (op === "||" && left.value) return booleanValue(true);
  const right = evalAst(rightAst, ctx, visited);
  if (isErrorValue(right)) return right;
  return right.kind === "boolean" ? booleanValue(right.value) : errorValue(`${op} expects boolean operands`);
}

function evalBinary(op: BinaryOp, leftAst: Ast, rightAst: Ast, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  if (op === "&&" || op === "||") return evalLogical(op, leftAst, rightAst, ctx, visited);

  const left = evalAst(leftAst, ctx, visited);
  if (isErrorValue(left)) return left;
  const right = evalAst(rightAst, ctx, visited);
  if (isErrorValue(right)) return right;

  if (op === "==" || op === "!=") {
    const nestedError = firstErrorDeep(left) ?? firstErrorDeep(right);
    if (nestedError) return nestedError;
    const equal = deepEqual(left, right);
    return booleanValue(op === "==" ? equal : !equal);
  }

  if (op === "<" || op === "<=" || op === ">" || op === ">=") return compareValues(op, left, right);

  if (op === "+" && left.kind === "text" && right.kind === "text") return textValue(left.value + right.value);
  if (left.kind === "number" && right.kind === "number") {
    if (op === "+") return numberValue(left.value + right.value);
    if (op === "-") return numberValue(left.value - right.value);
    if (op === "*") return numberValue(left.value * right.value);
    if (op === "/") return right.value === 0 ? errorValue("Division by zero") : numberValue(left.value / right.value);
    if (op === "%") return right.value === 0 ? errorValue("Division by zero") : numberValue(left.value % right.value);
  }

  if (left.kind === "date" && (op === "+" || op === "-")) {
    const duration = durationFromValue(right);
    if (duration) return addDuration(left, duration, op === "+" ? 1 : -1);
  }

  if (op === "+") return errorValue("+ expects number+number, text+text, or date+duration");
  if (op === "-") return errorValue("- expects number+number or date+duration");
  return errorValue(`${op} expects number operands`);
}

function evalCall(name: string, args: readonly Ast[], ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  if (name === "if") {
    if (args.length !== 3) return errorValue("if expects 3 arguments");
    const condition = evalAst(args[0], ctx, visited);
    if (isErrorValue(condition)) return condition;
    if (condition.kind !== "boolean") return errorValue("if condition expects boolean");
    return evalAst(condition.value ? args[1] : args[2], ctx, visited);
  }
  if (name === "isEmpty") {
    if (args.length !== 1) return errorValue("isEmpty expects 1 argument");
    const evaluated = evalArgs(args, ctx, visited);
    return Array.isArray(evaluated) ? isEmpty(evaluated[0]) : evaluated;
  }
  if (name === "now" || name === "today") {
    if (args.length !== 0) return errorValue(`${name} expects 0 arguments`);
    return name === "now" ? dateValueFromDate(ctx.now, true) : dateValueFromDate(ctx.now, false);
  }
  return errorValue(`Unknown function ${name}`);
}

function evalMember(object: Ast, name: string, args: readonly Ast[] | null, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  const target = evalAst(object, ctx, visited);
  if (isErrorValue(target)) return target;
  const evaluatedArgs = args == null ? null : evalArgs(args, ctx, visited);
  if (evaluatedArgs != null && !Array.isArray(evaluatedArgs)) return evaluatedArgs;
  const handler = MEMBER_TABLE[target.kind]?.[name];
  if (!handler) return errorValue(`Unknown ${args == null ? "property" : "method"} ${name} for ${typeName(target)}`);
  return handler(target, evaluatedArgs, ctx);
}

function evalAst(ast: Ast, ctx: FormulaEvalContext, visited: readonly string[]): FormulaValue {
  switch (ast.kind) {
    case "literal":
      if (typeof ast.value === "string") return textValue(ast.value);
      if (typeof ast.value === "number") return numberValue(ast.value);
      if (typeof ast.value === "boolean") return booleanValue(ast.value);
      return nullValue();
    case "field":
      return evalField(ast.name, ctx);
    case "formulaRef":
      return evalFormulaRef(ast.name, ctx, visited);
    case "unary":
      return evalUnary(ast.op, ast.expr, ctx, visited);
    case "binary":
      return evalBinary(ast.op, ast.left, ast.right, ctx, visited);
    case "call":
      return evalCall(ast.name, ast.args, ctx, visited);
    case "member":
      return evalMember(ast.object, ast.name, ast.args, ctx, visited);
  }
}

export function evaluate(ast: Ast, ctx: FormulaEvalContext): FormulaValue {
  return evalAst(ast, ctx, []);
}
