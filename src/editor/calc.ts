// A small, dependency-free calculator for ```calc blocks — Logseq parity.
//
// Behavior matches OG Logseq's calc (frontend/extensions/calc.cljc): each line
// shares an environment, `name = expr` assigns, and later lines reference the
// special `last`. Operators include right-associative `^`, negative terms,
// factorial, and number-literal `%` (literally ÷100, not percent-of). Functions
// are the grammar's sqrt/log/ln/exp/abs/trig/inverse-trig set, plus Tine's
// floor/ceil/round extensions (kept as a deliberate superset of calc.bnf so
// existing Tine graphs don't break; OG shows an error for these). PI and E.
// The upstream source is /aux/koutecky/logseq/og at 6e7afa8eb; precise semantic
// citations appear alongside each grammar/evaluation transcription below.

export interface CalcLine {
  input: string;
  /** Formatted result, or null for blank/comment/assignment-display/error. */
  output: string | null;
  error?: boolean;
}

/** If `text` is a ```calc fenced block, return its inner source (the lines
 *  between the fences); otherwise null. Tolerates a missing closing fence (the
 *  block is mid-edit) by taking everything after the opener — so the editor's
 *  live preview keeps working while you type. */
export function calcSource(text: string): string | null {
  const lines = text.split("\n");
  if ((lines[0]?.trim().toLowerCase() ?? "") !== "```calc") return null;
  const inner: string[] = [];
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === "```") break;
    inner.push(lines[i]);
  }
  return inner.join("\n");
}

/** Wrap calc expression lines back into a ```calc fenced block — the inverse of
 *  `calcSource`. The editor shows the user the fence-stripped expressions (like
 *  OG), so on commit we re-fence what they typed. */
export function wrapCalc(inner: string): string {
  return "```calc\n" + inner + "\n```";
}

/** Serialize the calc editor's visible buffer for an exit commit. Accept either
 *  the normal bare expression buffer or an already-fenced calc value, and always
 *  write one canonical ```calc fence. */
export function serializeCalcExitCommit(text: string): string {
  return wrapCalc(calcSource(text) ?? text);
}

// This is intentionally a local decimal implementation rather than a dependency.
// OG uses bignumber.js values and default division precision in its evaluator
// (/aux/koutecky/logseq/og/src/main/frontend/extensions/calc.cljc:41-117).
const TEN = 10n;
const DIVISION_PLACES = 20;

function pow10(places: number): bigint {
  return TEN ** BigInt(places);
}

type Special = "nan" | "infinity" | undefined;

/** A signed, finite base-10 coefficient, plus the two BigNumber special values. */
class Decimal {
  private constructor(
    readonly sign: -1 | 0 | 1,
    readonly coefficient: bigint,
    readonly scale: number,
    readonly special: Special = undefined,
  ) {}

  static finite(sign: number, coefficient: bigint, scale = 0): Decimal {
    if (coefficient === 0n) return new Decimal(0, 0n, 0);
    let normalized = coefficient < 0n ? -coefficient : coefficient;
    let normalizedScale = scale;
    while (normalizedScale > 0 && normalized % TEN === 0n) {
      normalized /= TEN;
      normalizedScale--;
    }
    return new Decimal(sign < 0 ? -1 : 1, normalized, normalizedScale);
  }

  static nan(): Decimal {
    return new Decimal(0, 0n, 0, "nan");
  }

  static infinity(sign: number): Decimal {
    return new Decimal(sign < 0 ? -1 : 1, 0n, 0, "infinity");
  }

  static parse(text: string): Decimal | null {
    const scientific = /^([0-9]*\.?[0-9]+)[eE]([+-]?\d+)$/.exec(text);
    if (scientific) {
      const exponent = Number(scientific[2]);
      if (!Number.isSafeInteger(exponent)) return null;
      const value = Decimal.parse(scientific[1]);
      if (!value) return null;
      if (value.scale > exponent) return Decimal.finite(value.sign, value.coefficient, value.scale - exponent);
      return Decimal.finite(value.sign, value.coefficient * pow10(exponent - value.scale));
    }
    if (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(text)) return null;
    const [whole = "", fraction = ""] = text.split(".");
    const digits = (whole || "0") + fraction;
    return Decimal.finite(1, BigInt(digits), fraction.length);
  }

  static fromNumber(value: number): Decimal {
    if (Number.isNaN(value)) return Decimal.nan();
    if (!Number.isFinite(value)) return Decimal.infinity(value);
    const parsed = Decimal.parse(Math.abs(value).toString());
    return parsed ? Decimal.finite(value < 0 ? -1 : 1, parsed.coefficient, parsed.scale) : Decimal.nan();
  }

  isFinite(): boolean {
    return !this.special;
  }

  isInteger(): boolean {
    return this.isFinite() && this.scale === 0;
  }

  isNegative(): boolean {
    return this.sign < 0;
  }

  private signedCoefficientAt(scale: number): bigint {
    const coefficient = this.coefficient * pow10(scale - this.scale);
    return this.sign < 0 ? -coefficient : coefficient;
  }

  private specialBinary(other: Decimal, operation: "add" | "multiply" | "divide"): Decimal | null {
    if (this.special === "nan" || other.special === "nan") return Decimal.nan();
    if (!this.special && !other.special) return null;
    if (operation === "add") {
      if (this.special && other.special && this.sign !== other.sign) return Decimal.nan();
      return this.special ? this : other;
    }
    if (operation === "multiply") {
      if ((!this.special && this.sign === 0) || (!other.special && other.sign === 0)) return Decimal.nan();
      return Decimal.infinity(this.sign * other.sign);
    }
    if (this.special && other.special) return Decimal.nan();
    if (this.special) return Decimal.infinity(this.sign * other.sign);
    return Decimal.finite(this.sign * other.sign, 0n);
  }

  plus(other: Decimal): Decimal {
    const special = this.specialBinary(other, "add");
    if (special) return special;
    const scale = Math.max(this.scale, other.scale);
    const coefficient = this.signedCoefficientAt(scale) + other.signedCoefficientAt(scale);
    return Decimal.finite(coefficient < 0n ? -1 : 1, coefficient < 0n ? -coefficient : coefficient, scale);
  }

  minus(other: Decimal): Decimal {
    return this.plus(other.negated());
  }

  multipliedBy(other: Decimal): Decimal {
    const special = this.specialBinary(other, "multiply");
    if (special) return special;
    return Decimal.finite(this.sign * other.sign, this.coefficient * other.coefficient, this.scale + other.scale);
  }

  dividedBy(other: Decimal): Decimal {
    if (!this.isFinite() || !other.isFinite()) {
      const special = this.specialBinary(other, "divide");
      if (special) return special;
    }
    if (other.coefficient === 0n) {
      return this.coefficient === 0n ? Decimal.nan() : Decimal.infinity(this.sign * other.sign || this.sign);
    }
    if (this.coefficient === 0n) return Decimal.finite(this.sign * other.sign, 0n);
    const exponent = other.scale - this.scale + DIVISION_PLACES;
    const numerator = exponent >= 0
      ? this.coefficient * pow10(exponent)
      : this.coefficient;
    const denominator = exponent >= 0
      ? other.coefficient
      : other.coefficient * pow10(-exponent);
    let quotient = numerator / denominator;
    const remainder = numerator % denominator;
    // BigNumber's default ROUNDING_MODE is ROUND_HALF_UP.
    if (remainder * 2n >= denominator) quotient++;
    return Decimal.finite(this.sign * other.sign, quotient, DIVISION_PLACES);
  }

  modulo(other: Decimal): Decimal {
    if (!this.isFinite() || !other.isFinite() || other.coefficient === 0n) return Decimal.nan();
    const scale = Math.max(this.scale, other.scale);
    const dividend = this.signedCoefficientAt(scale);
    const divisor = other.signedCoefficientAt(scale);
    const remainder = dividend % divisor;
    return Decimal.finite(remainder < 0n ? -1 : 1, remainder < 0n ? -remainder : remainder, scale);
  }

  negated(): Decimal {
    if (this.special === "nan" || this.sign === 0) return this;
    return this.special ? Decimal.infinity(-this.sign) : Decimal.finite(-this.sign, this.coefficient, this.scale);
  }

  abs(): Decimal {
    return this.sign < 0 ? this.negated() : this;
  }

  integerValue(): bigint | null {
    return this.isInteger() ? (this.sign < 0 ? -this.coefficient : this.coefficient) : null;
  }

  exponentiatedBy(other: Decimal): Decimal {
    const exponent = other.integerValue();
    if (exponent === null || !this.isFinite()) return Decimal.fromNumber(Math.pow(this.toNumber(), other.toNumber()));
    if (exponent === 0n) return Decimal.finite(1, 1n);
    const negativeExponent = exponent < 0n;
    let remaining = negativeExponent ? -exponent : exponent;
    let base: Decimal = this;
    let result = Decimal.finite(1, 1n);
    while (remaining > 0n) {
      if (remaining & 1n) result = result.multipliedBy(base);
      remaining >>= 1n;
      if (remaining > 0n) base = base.multipliedBy(base);
    }
    return negativeExponent ? Decimal.finite(1, 1n).dividedBy(result) : result;
  }

  squareRoot(): Decimal {
    if (!this.isFinite()) return this.special === "nan" || this.sign < 0 ? Decimal.nan() : this;
    if (this.sign < 0) return Decimal.nan();
    if (this.sign === 0) return this;
    const evenScale = this.scale % 2 === 0 ? this.scale : this.scale + 1;
    const adjusted = this.scale % 2 === 0 ? this.coefficient : this.coefficient * TEN;
    const workingPlaces = 40;
    const root = integerSquareRoot(adjusted * pow10(workingPlaces * 2));
    return Decimal.finite(1, root, workingPlaces + evenScale / 2);
  }

  precision(significantDigits: number): Decimal {
    if (!this.isFinite() || this.sign === 0) return this;
    const digits = this.coefficient.toString().length;
    if (digits <= significantDigits) return this;
    const drop = digits - significantDigits;
    let coefficient = this.coefficient / pow10(drop);
    if ((this.coefficient % pow10(drop)) * 2n >= pow10(drop)) coefficient++;
    return Decimal.finite(this.sign, coefficient, this.scale - drop);
  }

  private roundedCoefficient(scale: number): bigint {
    if (this.scale <= scale) return this.coefficient * pow10(scale - this.scale);
    const divisor = pow10(this.scale - scale);
    let coefficient = this.coefficient / divisor;
    if ((this.coefficient % divisor) * 2n >= divisor) coefficient++;
    return coefficient;
  }

  toFixed(places?: number): string {
    if (!this.isFinite()) return this.toString();
    if (places === undefined) return this.toPlain();
    const coefficient = this.roundedCoefficient(places);
    return this.withScale(coefficient, places);
  }

  toExponential(places?: number): string {
    if (!this.isFinite()) return this.toString();
    if (this.sign === 0) {
      const decimals = places === undefined ? "" : places === 0 ? "" : `.${"0".repeat(places)}`;
      return `0${decimals}e+0`;
    }
    const originalDigits = this.coefficient.toString();
    const wanted = places === undefined ? originalDigits.length : places + 1;
    let coefficient = this.coefficient;
    let exponent = this.exponent();
    const length = originalDigits.length;
    if (length > wanted) {
      const divisor = pow10(length - wanted);
      coefficient /= divisor;
      if ((this.coefficient % divisor) * 2n >= divisor) coefficient++;
      if (coefficient.toString().length > wanted) {
        coefficient /= TEN;
        exponent++;
      }
    } else if (length < wanted) {
      coefficient *= pow10(wanted - length);
    }
    const digits = coefficient.toString().padStart(wanted, "0");
    const mantissa = wanted === 1 ? digits : `${digits[0]}.${digits.slice(1)}`;
    return `${this.sign < 0 ? "-" : ""}${mantissa}e${exponent >= 0 ? "+" : ""}${exponent}`;
  }

  toPlain(): string {
    if (!this.isFinite()) return this.toString();
    if (this.sign === 0) return "0";
    const digits = this.coefficient.toString();
    const point = digits.length - this.scale;
    const body = point <= 0
      ? `0.${"0".repeat(-point)}${digits}`
      : point >= digits.length
        ? `${digits}${"0".repeat(point - digits.length)}`
        : `${digits.slice(0, point)}.${digits.slice(point)}`;
    return `${this.sign < 0 ? "-" : ""}${body}`;
  }

  toString(): string {
    if (this.special === "nan") return "NaN";
    if (this.special === "infinity") return this.sign < 0 ? "-Infinity" : "Infinity";
    return this.toPlain();
  }

  toNumber(): number {
    return Number(this.toString());
  }

  exponent(): number {
    return this.coefficient.toString().length - this.scale - 1;
  }

  toBase(base: 2 | 8 | 16): string {
    if (!this.isFinite()) return this.toString();
    const denominator = pow10(this.scale);
    let integer = this.coefficient / denominator;
    let remainder = this.coefficient % denominator;
    const digits = "0123456789abcdef";
    const fractional: number[] = [];
    for (let i = 0; i < 20 && remainder !== 0n; i++) {
      remainder *= BigInt(base);
      fractional.push(Number(remainder / denominator));
      remainder %= denominator;
    }
    if (fractional.length === 20 && remainder * 2n >= denominator) {
      for (let i = fractional.length - 1; i >= 0; i--) {
        if (fractional[i] < base - 1) {
          fractional[i]++;
          break;
        }
        fractional[i] = 0;
        if (i === 0) integer++;
      }
    }
    while (fractional[fractional.length - 1] === 0) fractional.pop();
    const prefix = base === 2 ? "0b" : base === 8 ? "0o" : "0x";
    const body = `${prefix}${integer.toString(base)}${fractional.length ? `.${fractional.map((digit) => digits[digit]).join("")}` : ""}`;
    return `${this.sign < 0 ? "-" : ""}${body}`;
  }

  private withScale(coefficient: bigint, scale: number): string {
    const digits = coefficient.toString().padStart(scale + 1, "0");
    const point = digits.length - scale;
    const body = scale === 0 ? digits : `${digits.slice(0, point)}.${digits.slice(point)}`;
    return `${this.sign < 0 ? "-" : ""}${body}`;
  }
}

function integerSquareRoot(value: bigint): bigint {
  if (value < 2n) return value;
  let current = 1n << BigInt(Math.ceil(value.toString(2).length / 2));
  for (;;) {
    const next = (current + value / current) >> 1n;
    if (next >= current) return current;
    current = next;
  }
}

// OG accepts these literal forms in grammar/calc.bnf:29-36.  The fractional
// forms of its non-decimal bases are finite decimal values because all bases
// are powers of two.
function parseBaseLiteral(text: string, base: 2 | 8 | 16): Decimal | null {
  const cleaned = text.replace(/,/g, "");
  const [whole = "", fraction = ""] = cleaned.slice(2).split(".");
  const read = (digits: string): bigint | null => {
    let result = 0n;
    for (const char of digits) {
      const digit = Number.parseInt(char, 16);
      if (!Number.isInteger(digit) || digit >= base) return null;
      result = result * BigInt(base) + BigInt(digit);
    }
    return result;
  };
  const integer = read(whole || "0");
  const fractional = fraction ? read(fraction) : 0n;
  if (integer === null || fractional === null) return null;
  if (!fraction) return Decimal.finite(1, integer);
  const binaryPlaces = fraction.length * (base === 2 ? 1 : base === 8 ? 3 : 4);
  return Decimal.finite(1, integer * pow10(binaryPlaces) + fractional * (5n ** BigInt(binaryPlaces)), binaryPlaces);
}

type NumberKind = "number" | "scientific" | "mixed";
type Tok =
  | { t: "num"; v: Decimal; kind: NumberKind }
  | { t: "name"; v: string }
  | { t: "op"; v: string }
  | { t: "("; spaced: boolean }
  | { t: ")" };

function tokenize(source: string): Tok[] | null {
  const toks: Tok[] = [];
  let i = 0;
  let spaced = false;
  while (i < source.length) {
    const char = source[i];
    if (/\s/.test(char)) {
      spaced = true;
      i++;
      continue;
    }
    if (char === "#") break;
    if (char === "(" || char === ")") {
      toks.push(char === "(" ? { t: "(", spaced } : { t: ")" });
      spaced = false;
      i++;
      continue;
    }
    if ("+-*/^!%".includes(char)) {
      toks.push({ t: "op", v: char });
      spaced = false;
      i++;
      continue;
    }
    if (/[0-9.]/.test(char)) {
      const rest = source.slice(i);
      const mixed = /^(\d+)\s+(\d+)\/(\d+)/.exec(rest);
      if (mixed) {
        const numerator = Decimal.finite(1, BigInt(mixed[2])).dividedBy(Decimal.finite(1, BigInt(mixed[3])));
        toks.push({ t: "num", v: Decimal.finite(1, BigInt(mixed[1])).plus(numerator), kind: "mixed" });
        i += mixed[0].length;
        spaced = false;
        continue;
      }
      const baseMatch = /^0([xob])/i.exec(rest);
      if (baseMatch) {
        const digit = baseMatch[1].toLowerCase();
        const alphabet = digit === "x" ? "0-9a-fA-F" : digit === "o" ? "0-7" : "01";
        const literal = new RegExp(`^0${digit}(?:[${alphabet}]+(?:,[${alphabet}]+)*(?:\\.[${alphabet}]*)?|[${alphabet}]*\\.[${alphabet}]+)`).exec(rest);
        if (!literal) return null;
        const value = parseBaseLiteral(literal[0], digit === "x" ? 16 : digit === "o" ? 8 : 2);
        if (!value) return null;
        toks.push({ t: "num", v: value, kind: "number" });
        i += literal[0].length;
        spaced = false;
        continue;
      }
      const scientific = /^(?:\d*\.?\d+)[eE][+-]?\d+/.exec(rest);
      const decimal = /^(?:\d+(?:,\d+)*(?:\.\d*)?|\d*\.\d+)/.exec(rest);
      const literal = scientific ?? decimal;
      if (!literal) return null;
      const value = Decimal.parse(literal[0].replace(/,/g, ""));
      if (!value) return null;
      toks.push({ t: "num", v: value, kind: scientific ? "scientific" : "number" });
      i += literal[0].length;
      spaced = false;
      continue;
    }
    if (/[A-Za-z_]/.test(char)) {
      const name = /^[A-Za-z_][A-Za-z0-9_]*/.exec(source.slice(i))![0];
      toks.push(name === "mod" ? { t: "op", v: "mod" } : { t: "name", v: name });
      i += name.length;
      spaced = false;
      continue;
    }
    return null;
  }
  return toks;
}

const CONSTS: Record<string, Decimal> = {
  PI: Decimal.parse("3.14159265358979323846")!,
  E: Decimal.parse("2.71828182845904523536")!,
};

const FAILURE = Symbol("calc failure");
type CalcValue = Decimal | typeof FAILURE;

interface CalcEnvironment {
  values: Map<string, CalcValue>;
  base?: "hex" | "oct" | "bin" | "decimal";
  mode?: "fix" | "sci" | "frac";
  places?: number;
  precision?: number;
  maxDenominator?: number;
  improper?: boolean;
}

// The production forms and precedence below transcribe OG's grammar exactly:
// /aux/koutecky/logseq/og/src/resources/grammar/calc.bnf:1-52.
class Parser {
  pos = 0;
  constructor(
    private readonly toks: Tok[],
    private readonly env: CalcEnvironment,
  ) {}

  peek(): Tok | undefined {
    return this.toks[this.pos];
  }

  expr(): Decimal {
    let value = this.term();
    for (;;) {
      const op = this.peek();
      if (!op || op.t !== "op" || (op.v !== "+" && op.v !== "-")) return value;
      this.pos++;
      const right = this.term();
      value = op.v === "+" ? value.plus(right) : value.minus(right);
    }
  }

  private term(): Decimal {
    let value = this.powerTerm();
    for (;;) {
      const op = this.peek();
      if (!op || op.t !== "op" || !["*", "/", "mod"].includes(op.v)) return value;
      this.pos++;
      const right = this.powerTerm();
      value = op.v === "*" ? value.multipliedBy(right) : op.v === "/" ? value.dividedBy(right) : value.modulo(right);
    }
  }

  private powerTerm(): Decimal {
    const op = this.peek();
    if (op?.t === "op" && op.v === "-") {
      this.pos++;
      return this.positivePowerTerm().negated();
    }
    return this.positivePowerTerm();
  }

  private positivePowerTerm(): Decimal {
    const base = this.posterm();
    const op = this.peek();
    if (op?.t === "op" && op.v === "^") {
      this.pos++;
      return base.exponentiatedBy(this.powerTerm());
    }
    if (op?.t === "op" && op.v === "!") {
      this.pos++;
      return factorial(base);
    }
    return base;
  }

  private posterm(): Decimal {
    const token = this.peek();
    if (!token) throw new Error("unexpected end");
    if (token.t === "num") {
      this.pos++;
      const percent = this.peek();
      if (token.kind === "number" && percent?.t === "op" && percent.v === "%") {
        this.pos++;
        return token.v.dividedBy(Decimal.finite(1, 100n));
      }
      return token.v;
    }
    if (token.t === "(") {
      this.pos++;
      const value = this.expr();
      if (this.peek()?.t !== ")") throw new Error("missing )");
      this.pos++;
      return value;
    }
    if (token.t !== "name") throw new Error("unexpected token");
    this.pos++;
    const next = this.peek();
    if (next?.t === "(" && !next.spaced) {
      this.pos++;
      const value = this.expr();
      if (this.peek()?.t !== ")") throw new Error("missing )");
      this.pos++;
      return applyFunction(token.v, value);
    }
    if (token.v in CONSTS) return CONSTS[token.v];
    const value = this.env.values.get(token.v);
    if (!value || value === FAILURE) throw new Error(`can't find variable ${token.v}`);
    return value;
  }
}

// OG accepts only non-negative integer factorial inputs below 254; its
// `isPositive` check includes BigNumber's positive zero, so 0! is 1
// (/aux/koutecky/logseq/og/src/main/frontend/extensions/calc.cljc:58-61).
function factorial(value: Decimal): Decimal {
  const n = value.integerValue();
  if (n === null || n < 0n || n >= 254n) return Decimal.nan();
  let result = Decimal.finite(1, 1n);
  for (let factor = 2n; factor <= n; factor++) result = result.multipliedBy(Decimal.finite(1, factor));
  return result;
}

function applyFunction(name: string, value: Decimal): Decimal {
  // sqrt/abs are BigNumber operations; the remaining transcendental functions
  // are explicitly delegated to JS Math then wrapped by OG (calc.cljc:62-92).
  if (name === "sqrt") return value.squareRoot();
  if (name === "abs") return value.abs();
  const fn: Record<string, (input: number) => number> = {
    log: Math.log10,
    ln: Math.log,
    exp: Math.exp,
    sin: Math.sin,
    cos: Math.cos,
    tan: Math.tan,
    atan: Math.atan,
    asin: Math.asin,
    acos: Math.acos,
    // Tine extension beyond OG calc.bnf:14-25 (deliberate superset; see header).
    floor: Math.floor,
    ceil: Math.ceil,
    round: Math.round,
  };
  const apply = fn[name];
  if (!apply) throw new Error(`unknown function ${name}`);
  return Decimal.fromNumber(apply(value.toNumber()));
}

type Directive =
  | { kind: "base"; base: "hex" | "oct" | "bin" | "decimal" }
  | { kind: "fix"; places: number }
  | { kind: "sci"; places?: number }
  | { kind: "normal"; precision?: number }
  | { kind: "frac"; maxDenominator?: number; improper: boolean };

function parseDirective(text: string): Directive | null {
  const source = text.trim();
  const base = /^:(hex(?:adecimal)?|dec(?:imal)?|oct(?:al)?|bin(?:ary)?)$/i.exec(source);
  if (base) {
    const name = base[1].toLowerCase();
    return { kind: "base", base: name.startsWith("hex") ? "hex" : name.startsWith("dec") ? "decimal" : name.startsWith("oct") ? "oct" : "bin" };
  }
  const format = /^:(?:format|fmt)\s+(.+)$/.exec(source);
  if (!format) return null;
  const body = format[1].trim();
  let match = /^fix(?:ed)?\s*(\d+)$/i.exec(body);
  if (match) return { kind: "fix", places: Number(match[1]) };
  match = /^sci(?:entific)?\s*(\d+)?$/i.exec(body);
  if (match) return { kind: "sci", places: match[1] === undefined ? undefined : Number(match[1]) };
  match = /^norm(?:al)?\s*(\d+)?$/i.exec(body);
  if (match) return { kind: "normal", precision: match[1] === undefined ? undefined : Number(match[1]) };
  match = /^frac(?:tions?)?\s*(\d+)?$/i.exec(body);
  if (match) return { kind: "frac", maxDenominator: match[1] === undefined ? undefined : Number(match[1]), improper: false };
  match = /^imp(?:roper)?\s*(\d+)?$/i.exec(body);
  return match ? { kind: "frac", maxDenominator: match[1] === undefined ? undefined : Number(match[1]), improper: true } : null;
}

// Directive state and formatting follow calc.cljc:91-110 and :175-203.  A
// directive returns the current `last`, so it is both stateful and displayable.
function applyDirective(env: CalcEnvironment, directive: Directive): CalcValue | undefined {
  if (directive.kind === "base") env.base = directive.base;
  else if (directive.kind === "fix") {
    env.mode = "fix";
    env.places = directive.places;
  } else if (directive.kind === "sci") {
    env.mode = "sci";
    env.places = directive.places;
  } else if (directive.kind === "normal") {
    env.mode = undefined;
    env.places = undefined;
    env.precision = directive.precision;
  } else {
    env.mode = "frac";
    env.maxDenominator = directive.maxDenominator;
    env.improper = directive.improper;
  }
  return env.values.get("last");
}

function formatNormal(env: CalcEnvironment, value: Decimal): string {
  if (!value.isFinite()) return value.toString();
  const precision = env.precision ?? 21;
  const display = value.precision(precision);
  const canFit = display.sign === 0 || (display.exponent() < precision && display.scale <= precision + 1);
  return canFit ? display.toFixed() : display.toExponential();
}

function canFix(value: Decimal, places: number): boolean {
  if (!value.isFinite() || value.sign === 0) return value.isFinite();
  // Equivalent to OG's 0.5 × 10^-places <= |value| < 1e21 check.
  const largeEnough = value.coefficient * pow10(places + 1) >= 5n * pow10(value.scale);
  return largeEnough && value.exponent() < 21;
}

function closestFraction(value: Decimal, maximum: number): [bigint, bigint] | null {
  if (!value.isFinite() || maximum < 1) return null;
  const negative = value.sign < 0;
  let numerator = value.coefficient;
  let denominator = pow10(value.scale);
  let previousNumerator = 0n;
  let previousDenominator = 1n;
  let currentNumerator = 1n;
  let currentDenominator = 0n;
  const limit = BigInt(maximum);
  while (denominator !== 0n) {
    const quotient = numerator / denominator;
    const nextNumerator = previousNumerator + quotient * currentNumerator;
    const nextDenominator = previousDenominator + quotient * currentDenominator;
    if (nextDenominator > limit) {
      const scale = currentDenominator === 0n ? 0n : (limit - previousDenominator) / currentDenominator;
      const candidateNumerator = previousNumerator + scale * currentNumerator;
      const candidateDenominator = previousDenominator + scale * currentDenominator;
      const error = (a: bigint, b: bigint) => {
        const signed = a * pow10(value.scale) - value.coefficient * b;
        return signed < 0n ? -signed : signed;
      };
      const useCandidate = candidateDenominator > 0n && (currentDenominator === 0n
        || error(candidateNumerator, candidateDenominator) * currentDenominator
          <= error(currentNumerator, currentDenominator) * candidateDenominator);
      const bestNumerator = useCandidate ? candidateNumerator : currentNumerator;
      const bestDenominator = useCandidate ? candidateDenominator : currentDenominator;
      return bestDenominator === 0n ? null : [negative ? -bestNumerator : bestNumerator, bestDenominator];
    }
    previousNumerator = currentNumerator;
    previousDenominator = currentDenominator;
    currentNumerator = nextNumerator;
    currentDenominator = nextDenominator;
    const remainder = numerator % denominator;
    numerator = denominator;
    denominator = remainder;
  }
  return [negative ? -currentNumerator : currentNumerator, currentDenominator];
}

function formatFraction(env: CalcEnvironment, value: Decimal): string {
  const fraction = closestFraction(value, env.maxDenominator ?? 4095);
  if (!fraction) return formatNormal(env, value);
  const [numerator, denominator] = fraction;
  const reconstructed = Decimal.finite(numerator < 0n ? -1 : 1, numerator < 0n ? -numerator : numerator).dividedBy(Decimal.finite(1, denominator));
  const delta = reconstructed.minus(value).abs();
  if (delta.sign !== 0 && delta.exponent() >= -16) return formatNormal(env, value);
  if (denominator === 1n) return formatNormal(env, Decimal.finite(numerator < 0n ? -1 : 1, numerator < 0n ? -numerator : numerator));
  if (env.improper) return `${numerator}/${denominator}`;
  const whole = numerator / denominator;
  const remainder = numerator % denominator;
  if (whole === 0n) return `${numerator}/${denominator}`;
  return `${whole} ${remainder < 0n ? -remainder : remainder}/${denominator}`;
}

function formatValue(env: CalcEnvironment, value: Decimal): string {
  if (env.base === "hex") return value.toBase(16);
  if (env.base === "oct") return value.toBase(8);
  if (env.base === "bin") return value.toBase(2);
  if (env.mode === "fix") return canFix(value, env.places ?? 0) ? value.toFixed(env.places) : value.toExponential(env.places);
  if (env.mode === "sci") return value.toExponential(env.places);
  if (env.mode === "frac") return formatFraction(env, value);
  return formatNormal(env, value);
}

/** Evaluate a multi-line calc source; returns one entry per input line. */
export function evalCalc(src: string): CalcLine[] {
  const env: CalcEnvironment = { values: new Map() };
  return src.split("\n").map((line) => {
    const noComment = line.split("#")[0];
    if (!noComment.trim()) return { input: line, output: null };
    try {
      let value: CalcValue | undefined;
      if (noComment.trimStart().startsWith(":")) {
        const directive = parseDirective(noComment);
        if (!directive) throw new Error("invalid directive");
        value = applyDirective(env, directive);
      } else {
        const assignment = /^\s*(_*[A-Za-z]+[_A-Za-z0-9]*)\s*=\s*(.*)$/.exec(noComment);
        const tokens = tokenize(assignment ? assignment[2] : noComment);
        if (!tokens || tokens.length === 0) throw new Error("invalid expression");
        const parser = new Parser(tokens, env);
        value = parser.expr();
        if (parser.pos !== tokens.length) throw new Error("trailing input");
        if (assignment) {
          if (assignment[1] in CONSTS) throw new Error("cannot reassign constant");
          env.values.set(assignment[1], value);
        }
      }
      // `eval-lines` calls assign-last-value for parse/evaluation failures too;
      // that makes a subsequent `last` another failure (calc.cljc:209-215).
      if (value !== undefined) env.values.set("last", value);
      if (value === undefined) return { input: line, output: null };
      if (value === FAILURE) return { input: line, output: null, error: true };
      return { input: line, output: formatValue(env, value) };
    } catch {
      env.values.set("last", FAILURE);
      return { input: line, output: null, error: true };
    }
  });
}
