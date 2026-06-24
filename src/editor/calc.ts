// A small, dependency-free calculator for ```calc blocks — Logseq parity.
//
// Behavior matches OG Logseq's calc (frontend/extensions/calc.cljc), the parts
// people actually use: each line is parsed + evaluated independently sharing one
// variable env; `name = expr` assigns; later lines reference earlier variables
// and the special `last` (previous line's result). Operators `+ - * / mod ^`
// (^ right-assoc), unary minus, parentheses, factorial `!`, and — IMPORTANTLY —
// `X%` means literally `X / 100` (so `100 + 10%` = 100.1, NOT 110; this is OG's
// only percent semantics). Functions: sqrt log(=log10) ln exp abs round floor
// ceil sin cos tan. Constants PI, E. Commas in numbers are stripped. `#` starts
// a line comment. Blank / comment-only / unparseable lines produce no output.
//
// Pure + unit-tested (see calc.test.ts). Uses JS numbers (a notepad calculator,
// not arbitrary precision) — adequate for the common cases; OG uses bignumber.js.

export interface CalcLine {
  input: string;
  /** Formatted result, or null for blank/comment/assignment-display/error. */
  output: string | null;
  error?: boolean;
}

type Tok =
  | { t: "num"; v: number }
  | { t: "name"; v: string }
  | { t: "op"; v: string }
  | { t: "(" }
  | { t: ")" };

const FUNCS: Record<string, (x: number) => number> = {
  sqrt: Math.sqrt,
  abs: Math.abs,
  ln: Math.log,
  log: Math.log10,
  exp: Math.exp,
  round: Math.round,
  floor: Math.floor,
  ceil: Math.ceil,
  sin: Math.sin,
  cos: Math.cos,
  tan: Math.tan,
};
const CONSTS: Record<string, number> = { PI: Math.PI, E: Math.E };

function tokenize(s: string): Tok[] | null {
  const toks: Tok[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === " " || c === "\t") {
      i++;
      continue;
    }
    if (c === "#") break; // line comment to end of line
    if (c === "(" || c === ")") {
      toks.push({ t: c });
      i++;
      continue;
    }
    if ("+-*/^!%".includes(c)) {
      toks.push({ t: "op", v: c });
      i++;
      continue;
    }
    // number: digits, decimal point, commas (stripped), scientific notation
    if (/[0-9.]/.test(c)) {
      let j = i;
      while (j < s.length && /[0-9.,]/.test(s[j])) j++;
      // optional exponent
      if (s[j] === "e" || s[j] === "E") {
        j++;
        if (s[j] === "+" || s[j] === "-") j++;
        while (j < s.length && /[0-9]/.test(s[j])) j++;
      }
      const raw = s.slice(i, j).replace(/,/g, "");
      const v = Number(raw);
      if (!Number.isFinite(v)) return null;
      toks.push({ t: "num", v });
      i = j;
      continue;
    }
    // identifier (name / function / constant / `mod`)
    if (/[a-zA-Z_]/.test(c)) {
      let j = i;
      while (j < s.length && /[a-zA-Z_0-9]/.test(s[j])) j++;
      const name = s.slice(i, j);
      if (name === "mod") toks.push({ t: "op", v: "mod" });
      else toks.push({ t: "name", v: name });
      i = j;
      continue;
    }
    return null; // unexpected character
  }
  return toks;
}

// Recursive-descent / precedence-climbing parser+evaluator over the token list.
class Parser {
  pos = 0;
  constructor(
    private toks: Tok[],
    private env: Map<string, number>,
  ) {}
  peek(): Tok | undefined {
    return this.toks[this.pos];
  }
  // expr: addition/subtraction
  expr(): number {
    let v = this.term();
    for (;;) {
      const t = this.peek();
      if (t && t.t === "op" && (t.v === "+" || t.v === "-")) {
        this.pos++;
        const r = this.term();
        v = t.v === "+" ? v + r : v - r;
      } else break;
    }
    return v;
  }
  // term: * / mod
  term(): number {
    let v = this.power();
    for (;;) {
      const t = this.peek();
      if (t && t.t === "op" && (t.v === "*" || t.v === "/" || t.v === "mod")) {
        this.pos++;
        const r = this.power();
        v = t.v === "*" ? v * r : t.v === "/" ? v / r : v % r;
      } else break;
    }
    return v;
  }
  // power: ^ (right-assoc), over the postfix/percent/factorial layer
  power(): number {
    const base = this.postfix();
    const t = this.peek();
    if (t && t.t === "op" && t.v === "^") {
      this.pos++;
      return Math.pow(base, this.power());
    }
    return base;
  }
  // postfix: `%` (÷100) and `!` (factorial) bind tighter than ^'s base
  postfix(): number {
    let v = this.unary();
    for (;;) {
      const t = this.peek();
      if (t && t.t === "op" && t.v === "%") {
        this.pos++;
        v = v / 100;
      } else if (t && t.t === "op" && t.v === "!") {
        this.pos++;
        v = factorial(v);
      } else break;
    }
    return v;
  }
  unary(): number {
    const t = this.peek();
    if (t && t.t === "op" && t.v === "-") {
      this.pos++;
      return -this.unary();
    }
    if (t && t.t === "op" && t.v === "+") {
      this.pos++;
      return this.unary();
    }
    return this.atom();
  }
  atom(): number {
    const t = this.peek();
    if (!t) throw new Error("unexpected end");
    if (t.t === "num") {
      this.pos++;
      return t.v;
    }
    if (t.t === "(") {
      this.pos++;
      const v = this.expr();
      const close = this.peek();
      if (!close || close.t !== ")") throw new Error("missing )");
      this.pos++;
      return v;
    }
    if (t.t === "name") {
      this.pos++;
      // function call: name(...)
      if (this.peek()?.t === "(") {
        const fn = FUNCS[t.v];
        if (!fn) throw new Error(`unknown function ${t.v}`);
        this.pos++;
        const arg = this.expr();
        const close = this.peek();
        if (!close || close.t !== ")") throw new Error("missing )");
        this.pos++;
        return fn(arg);
      }
      if (t.v in CONSTS) return CONSTS[t.v];
      const v = this.env.get(t.v);
      if (v === undefined) throw new Error(`unknown variable ${t.v}`);
      return v;
    }
    throw new Error("unexpected token");
  }
}

function factorial(n: number): number {
  if (n < 0 || !Number.isInteger(n)) return NaN;
  let r = 1;
  for (let k = 2; k <= n; k++) r *= k;
  return r;
}

/** Format a result like OG: integers bare, otherwise trim trailing zeros. */
function fmt(v: number): string {
  if (!Number.isFinite(v)) return String(v); // Infinity / NaN
  if (Number.isInteger(v)) return String(v);
  // up to 10 significant decimals, trimmed
  return parseFloat(v.toFixed(10)).toString();
}

/** Evaluate a multi-line calc source; returns one entry per input line. */
export function evalCalc(src: string): CalcLine[] {
  const env = new Map<string, number>();
  return src.split("\n").map((line) => {
    const noComment = line.split("#")[0];
    if (!noComment.trim()) return { input: line, output: null };
    // assignment?  name = expr
    const m = /^\s*([a-zA-Z_][a-zA-Z_0-9]*)\s*=(?!=)\s*(.*)$/.exec(noComment);
    const exprText = m ? m[2] : noComment;
    const toks = tokenize(exprText);
    if (!toks || toks.length === 0) return { input: line, output: null };
    try {
      const p = new Parser(toks, env);
      const v = p.expr();
      if (p.pos !== toks.length) throw new Error("trailing input");
      if (m) {
        if (m[1] in CONSTS) throw new Error("cannot reassign constant");
        env.set(m[1], v);
      }
      env.set("last", v);
      return { input: line, output: fmt(v) };
    } catch {
      return { input: line, output: null, error: true };
    }
  });
}
