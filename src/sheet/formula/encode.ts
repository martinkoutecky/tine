export function formulaNameValid(name: string): boolean {
  return /^[a-z0-9-]+$/.test(name);
}

function encodeParenArmor(src: string): string {
  let out = "";
  for (let i = 0; i < src.length; i += 1) {
    const ch = src[i];
    out += ch;
    if (ch !== "(") continue;
    let j = i + 1;
    while (src[j] === " ") j += 1;
    if (src[j] === "(") out += " ";
  }
  return out;
}

function decodeParenArmor(src: string): string {
  let out = "";
  for (let i = 0; i < src.length; i += 1) {
    const ch = src[i];
    out += ch;
    if (ch !== "(" || src[i + 1] !== " ") continue;
    let j = i + 1;
    while (src[j] === " ") j += 1;
    if (src[j] === "(") {
      out += " ".repeat(j - i - 2);
      i = j - 1;
    }
  }
  return out;
}

function encodeHashArmor(src: string): string {
  let out = "";
  let quote: "'" | "\"" | null = null;
  let escaped = false;

  for (let i = 0; i < src.length; i += 1) {
    const ch = src[i];
    if (!quote) {
      out += ch;
      if (ch === "'" || ch === "\"") quote = ch;
      continue;
    }

    out += ch === "#" ? "\\#" : ch;
    if (escaped) escaped = false;
    else if (ch === "\\") escaped = true;
    else if (ch === quote) quote = null;
  }

  return out;
}

function decodeHashArmor(src: string): string {
  let out = "";
  let quote: "'" | "\"" | null = null;
  let escaped = false;

  for (let i = 0; i < src.length; i += 1) {
    const ch = src[i];
    if (!quote) {
      out += ch;
      if (ch === "'" || ch === "\"") quote = ch;
      continue;
    }

    if (ch === "#" && out.endsWith("\\")) out = out.slice(0, -1);
    out += ch;

    if (escaped) escaped = false;
    else if (ch === "\\") escaped = true;
    else if (ch === quote) quote = null;
  }

  return out;
}

export function encodeFormulaExpr(expr: string): string {
  // `( (` is the serialized block-ref armor for a raw `((`. A user's real
  // spaces between opening parens are escaped by adding one extra space; decode
  // subtracts exactly one only when another `(` follows.
  return encodeHashArmor(encodeParenArmor(expr));
}

export function decodeFormulaExpr(line: string): string {
  return decodeParenArmor(decodeHashArmor(line));
}
