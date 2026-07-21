// Bounded, data-only Hiccup transcription for configured macro expansions.
// OG safe-reads Hiccup, serializes it, then sanitizes the HTML at
// /aux/koutecky/logseq/og/src/main/frontend/components/block.cljs:1554-1562.
// This module deliberately implements only the frozen supported subset; it never
// evaluates ClojureScript, and its output is still untrusted until DOMPurify runs.

const MAX_SOURCE_BYTES = 64 * 1024;
const MAX_DEPTH = 64;
const MAX_NODES = 2048;
const TOKEN = /^[A-Za-z][A-Za-z0-9-]*$/;

class HiccupParseError extends Error {}

type AttrValue = { kind: "supported"; value: string } | { kind: "unsupported" };

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

class Reader {
  private index = 0;
  private nodes = 0;

  constructor(private readonly source: string) {}

  parse(): string {
    this.skipWhitespace();
    if (this.peek() !== "[") this.fail();
    const html = this.parseElement(1);
    this.skipWhitespace();
    if (this.index !== this.source.length) this.fail();
    return html;
  }

  private parseElement(depth: number): string {
    this.checkDepth(depth);
    this.bumpNode();
    this.expect("[");
    this.skipWhitespace();
    if (this.peek() !== ":") this.fail();
    const tagSpec = this.readKeyword();
    const { tag, id, classes } = this.parseTagSpec(tagSpec);
    this.skipWhitespace();

    const attrs = new Map<string, string>();
    if (id) attrs.set("id", id);
    if (classes.length) attrs.set("class", classes.join(" "));
    if (this.peek() === "{") {
      for (const [name, value] of this.parseAttrs(depth + 1)) {
        if (name === "class" && attrs.has("class")) {
          attrs.set("class", `${attrs.get("class")} ${value}`);
        } else {
          attrs.set(name, value);
        }
      }
    }

    let children = "";
    for (;;) {
      this.skipWhitespace();
      if (this.peek() === "]") break;
      if (this.index >= this.source.length) this.fail();
      children += this.parseChild(depth);
    }
    this.expect("]");

    let serializedAttrs = "";
    for (const [name, value] of attrs) {
      serializedAttrs += ` ${name}="${escapeHtml(value)}"`;
    }
    return `<${tag}${serializedAttrs}>${children}</${tag}>`;
  }

  private parseChild(parentDepth: number): string {
    const ch = this.peek();
    if (ch === '"') {
      this.bumpNode();
      return escapeHtml(this.readString());
    }
    if (ch === "[") return this.parseElement(parentDepth + 1);
    if (ch === "(") return this.parseSeq(parentDepth + 1);
    if (this.startsNumber()) {
      this.bumpNode();
      return escapeHtml(this.readNumber());
    }

    // Consume one complete reader form before rejecting it. This makes malformed
    // and unsupported forms deterministic whole-conversion fallbacks.
    this.parseGeneric(parentDepth + 1);
    this.fail();
  }

  private parseSeq(depth: number): string {
    this.checkDepth(depth);
    this.bumpNode();
    this.expect("(");
    let children = "";
    for (;;) {
      this.skipWhitespace();
      if (this.peek() === ")") break;
      if (this.index >= this.source.length) this.fail();
      children += this.parseChild(depth);
    }
    this.expect(")");
    return children;
  }

  private parseAttrs(depth: number): Array<[string, string]> {
    this.checkDepth(depth);
    this.expect("{");
    const attrs: Array<[string, string]> = [];
    for (;;) {
      this.skipWhitespace();
      if (this.peek() === "}") break;
      if (this.index >= this.source.length) this.fail();
      const name = this.readAttrName();
      if (!TOKEN.test(name)) this.fail();
      this.skipWhitespace();
      if (this.peek() === "}" || this.index >= this.source.length) this.fail();
      const value = this.readAttrValue(depth);
      if (value.kind === "supported") attrs.push([name, value.value]);
    }
    this.expect("}");
    return attrs;
  }

  private readAttrName(): string {
    if (this.peek() === ":") return this.readKeyword();
    if (this.peek() === '"') return this.readString();
    this.fail();
  }

  private readAttrValue(parentDepth: number): AttrValue {
    if (this.peek() === '"') return { kind: "supported", value: this.readString() };
    if (this.startsNumber()) return { kind: "supported", value: this.readNumber() };
    this.parseGeneric(parentDepth + 1);
    return { kind: "unsupported" };
  }

  private parseTagSpec(spec: string): { tag: string; id?: string; classes: string[] } {
    const parts = spec.split(/([#.])/);
    const tag = parts.shift() ?? "";
    if (!TOKEN.test(tag)) this.fail();
    let id: string | undefined;
    const classes: string[] = [];
    while (parts.length) {
      const separator = parts.shift();
      const value = parts.shift() ?? "";
      if (!TOKEN.test(value)) this.fail();
      if (separator === "#") {
        if (id !== undefined) this.fail();
        id = value;
      } else if (separator === ".") {
        classes.push(value);
      } else {
        this.fail();
      }
    }
    return { tag, id, classes };
  }

  /** Consume a complete reader form without treating it as renderable data. */
  private parseGeneric(depth: number): void {
    this.checkDepth(depth);
    this.bumpNode();
    this.skipWhitespace();
    const ch = this.peek();
    if (ch === '"') {
      this.readString();
      return;
    }
    if (ch === ":") {
      this.readKeyword();
      return;
    }
    if (ch === "[") {
      this.parseGenericCollection("[", "]", depth);
      return;
    }
    if (ch === "(") {
      this.parseGenericCollection("(", ")", depth);
      return;
    }
    if (ch === "{") {
      this.parseGenericMap(depth);
      return;
    }
    if (ch === "#") {
      this.index++;
      if (this.peek() === "{") {
        this.parseGenericCollection("{", "}", depth);
        return;
      }
      const tag = this.readAtom();
      if (!tag) this.fail();
      this.skipWhitespace();
      if (this.index >= this.source.length) this.fail();
      this.parseGeneric(depth + 1);
      return;
    }
    if (ch === "'") {
      this.index++;
      this.parseGeneric(depth + 1);
      return;
    }
    if (!this.readAtom()) this.fail();
  }

  private parseGenericCollection(open: string, close: string, depth: number): void {
    this.checkDepth(depth);
    this.expect(open);
    for (;;) {
      this.skipWhitespace();
      if (this.peek() === close) break;
      if (this.index >= this.source.length) this.fail();
      this.parseGeneric(depth + 1);
    }
    this.expect(close);
  }

  private parseGenericMap(depth: number): void {
    this.checkDepth(depth);
    this.expect("{");
    let forms = 0;
    for (;;) {
      this.skipWhitespace();
      if (this.peek() === "}") break;
      if (this.index >= this.source.length) this.fail();
      this.parseGeneric(depth + 1);
      forms++;
    }
    if (forms % 2 !== 0) this.fail();
    this.expect("}");
  }

  private readKeyword(): string {
    this.expect(":");
    const value = this.readAtom();
    if (!value) this.fail();
    return value;
  }

  private readString(): string {
    this.expect('"');
    let value = "";
    while (this.index < this.source.length) {
      const ch = this.source[this.index++];
      if (ch === '"') return value;
      if (ch !== "\\") {
        value += ch;
        continue;
      }
      if (this.index >= this.source.length) this.fail();
      const escaped = this.source[this.index++];
      switch (escaped) {
        case '"': value += '"'; break;
        case "\\": value += "\\"; break;
        case "n": value += "\n"; break;
        case "r": value += "\r"; break;
        case "t": value += "\t"; break;
        case "b": value += "\b"; break;
        case "f": value += "\f"; break;
        case "u": {
          const hex = this.source.slice(this.index, this.index + 4);
          if (!/^[0-9A-Fa-f]{4}$/.test(hex)) this.fail();
          value += String.fromCharCode(Number.parseInt(hex, 16));
          this.index += 4;
          break;
        }
        default: this.fail();
      }
    }
    this.fail();
  }

  private startsNumber(): boolean {
    const token = this.peekAtom();
    return /^[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?$/.test(token);
  }

  private readNumber(): string {
    const token = this.readAtom();
    if (!/^[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?$/.test(token)) this.fail();
    return token;
  }

  private peekAtom(): string {
    let end = this.index;
    while (end < this.source.length && !/[\s,\[\]{}()"']/.test(this.source[end])) end++;
    return this.source.slice(this.index, end);
  }

  private readAtom(): string {
    const value = this.peekAtom();
    this.index += value.length;
    return value;
  }

  private skipWhitespace(): void {
    while (this.index < this.source.length && /[\s,]/.test(this.source[this.index])) this.index++;
  }

  private peek(): string | undefined {
    return this.source[this.index];
  }

  private expect(ch: string): void {
    if (this.source[this.index] !== ch) this.fail();
    this.index++;
  }

  private checkDepth(depth: number): void {
    if (depth > MAX_DEPTH) this.fail();
  }

  private bumpNode(): void {
    this.nodes++;
    if (this.nodes > MAX_NODES) this.fail();
  }

  private fail(): never {
    throw new HiccupParseError();
  }
}

/**
 * Convert the supported Hiccup subset to an escaped HTML string. `null` means
 * the caller must display the entire original source literally.
 */
export function hiccupToHtml(source: string): string | null {
  if (new TextEncoder().encode(source).byteLength > MAX_SOURCE_BYTES) return null;
  try {
    return new Reader(source).parse();
  } catch {
    return null;
  }
}
