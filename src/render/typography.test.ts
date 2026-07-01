import { describe, it, expect } from "vitest";
import { typographic } from "./typography";

describe("typographic replacements", () => {
  it("maps the core arrows and dashes", () => {
    expect(typographic("a -> b")).toBe("a → b");
    expect(typographic("a <- b")).toBe("a ← b");
    expect(typographic("a <-> b")).toBe("a ⟷ b");
    expect(typographic("a => b")).toBe("a ⇒ b");
    expect(typographic("pages 1--5")).toBe("pages 1–5"); // en dash
    expect(typographic("a --- b")).toBe("a — b"); // em dash
  });

  it("longest-match-first: combined runs map to one glyph, not piecemeal", () => {
    expect(typographic("a --> b")).toBe("a ⟶ b"); // NOT "a –> b"
    expect(typographic("a <-- b")).toBe("a ⟵ b");
    expect(typographic("a <--> b")).toBe("a ⟺ b");
  });

  it("works without surrounding spaces", () => {
    expect(typographic("if(x->y)")).toBe("if(x→y)");
    expect(typographic("1--2--3")).toBe("1–2–3");
  });

  it("is idempotent (glyphs contain no triggers)", () => {
    const once = typographic("a -> b -- c");
    expect(typographic(once)).toBe(once);
  });

  it("leaves untriggered text alone", () => {
    expect(typographic("just some words")).toBe("just some words");
    expect(typographic("a - b")).toBe("a - b"); // single hyphen unchanged
    expect(typographic("")).toBe("");
  });
});
