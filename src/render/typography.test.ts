import { describe, it, expect } from "vitest";
import { typographic, typoTypeReplace } from "./typography";

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

describe('typoTypeReplace ("on type" mode)', () => {
  it("replaces `>`-terminated arrows the moment `>` is typed (longest match)", () => {
    expect(typoTypeReplace("a->", 3, ">")).toEqual({ value: "a→", caret: 2 });
    expect(typoTypeReplace("x=>", 3, ">")).toEqual({ value: "x⇒", caret: 2 });
    expect(typoTypeReplace("a-->", 4, ">")).toEqual({ value: "a⟶", caret: 2 }); // NOT "a–>"
    expect(typoTypeReplace("<->", 3, ">")).toEqual({ value: "⟷", caret: 1 });
    expect(typoTypeReplace("<-->", 4, ">")).toEqual({ value: "⟺", caret: 1 });
  });

  it("does NOT fire mid-sequence when the next char could extend the run", () => {
    // Typing `--` must not become `–` — a following `-`/`>` might make `---`/`-->`.
    expect(typoTypeReplace("--", 2, "-")).toBeNull();
    expect(typoTypeReplace("<-", 2, "-")).toBeNull();
  });

  it("resolves deferred dash/left-arrow runs on the following boundary char", () => {
    expect(typoTypeReplace("-- ", 3, " ")).toEqual({ value: "– ", caret: 2 }); // en dash
    expect(typoTypeReplace("---x", 4, "x")).toEqual({ value: "—x", caret: 2 }); // em dash
    expect(typoTypeReplace("<-x", 3, "x")).toEqual({ value: "←x", caret: 2 });
    expect(typoTypeReplace("<--x", 4, "x")).toEqual({ value: "⟵x", caret: 2 });
  });

  it("leaves the source alone inside inline code (odd backticks before caret)", () => {
    expect(typoTypeReplace("`->", 3, ">")).toBeNull();
    // …but a closed `code` span is fine (even backticks).
    expect(typoTypeReplace("`x`->", 5, ">")).toEqual({ value: "`x`→", caret: 4 });
  });

  it("returns null when nothing completes at the caret", () => {
    expect(typoTypeReplace("hello", 5, "o")).toBeNull();
    expect(typoTypeReplace("a-", 2, "-")).toBeNull();
  });
});
