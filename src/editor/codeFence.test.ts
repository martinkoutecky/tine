import { describe, expect, it } from "vitest";
import { codeBodyExitTrim, codeBodyJoin, codeBodyProjection, codeFenceOnly } from "./codeFence";

describe("codeFenceOnly", () => {
  it("detects a whole-block markdown fence with a language", () => {
    expect(codeFenceOnly("```js\nconst x = 1;\nconsole.log(x);\n```", "md")).toEqual({ lang: "js" });
  });

  it("detects a fence with no language and tolerates missing closing fence (still typing)", () => {
    expect(codeFenceOnly("```\nraw code", "md")).toEqual({ lang: "" });
    expect(codeFenceOnly("```python\nx = 1", "md")).toEqual({ lang: "python" });
  });

  it("detects a tilde fence and an empty code body", () => {
    expect(codeFenceOnly("~~~sh\necho hi\n~~~", "md")).toEqual({ lang: "sh" });
    expect(codeFenceOnly("```\n```", "md")).toEqual({ lang: "" });
  });

  it("excludes calc fences (they have their own editor mode)", () => {
    expect(codeFenceOnly("```calc\n1 + 1\n```", "md")).toBeNull();
    expect(codeFenceOnly("```CALC\n1 + 1\n```", "md")).toBeNull();
  });

  it("rejects mixed content: text around the fence", () => {
    expect(codeFenceOnly("intro\n```js\nx\n```", "md")).toBeNull();
    expect(codeFenceOnly("```js\nx\n```\ntailing note", "md")).toBeNull();
    // ...but trailing BLANK lines are tolerated while composing.
    expect(codeFenceOnly("```js\nx\n```\n", "md")).toEqual({ lang: "js" });
  });

  it("rejects a second construct after a closed fence and fence-like inline code", () => {
    expect(codeFenceOnly("```js\nx\n```\n```py\ny", "md")).toBeNull();
    // An inner code line wrapped in single backticks is not a closer.
    expect(codeFenceOnly("```\n`weird`\nmore", "md")).toEqual({ lang: "" });
    expect(codeFenceOnly("`plain inline` no fence", "md")).toBeNull();
  });

  it("detects org #+BEGIN_SRC blocks", () => {
    expect(codeFenceOnly("#+BEGIN_SRC python\nx = 1\n#+END_SRC", "org")).toEqual({ lang: "python" });
    expect(codeFenceOnly("#+begin_src elisp\n(x 1)\n#+end_src", "org")).toEqual({ lang: "elisp" });
    expect(codeFenceOnly("#+BEGIN_SRC rust\nlet x = 1;", "org")).toEqual({ lang: "rust" });
  });

  it("detects org #+BEGIN_EXAMPLE blocks", () => {
    expect(codeFenceOnly("#+BEGIN_EXAMPLE\nstuff\n#+END_EXAMPLE", "org")).toEqual({ lang: "" });
    // An unclosed example block is still code-shaped (typing tolerance), same
    // as an unclosed markdown fence.
    expect(codeFenceOnly("#+BEGIN_EXAMPLE\nstuff\ntailing", "org")).toEqual({ lang: "" });
  });

  it("org src with text after the closer is not code-shaped", () => {
    expect(codeFenceOnly("#+BEGIN_SRC python\nx\n#+END_SRC\n/italic note/", "org")).toBeNull();
  });
});

// GH #412/#413 shared contract: a PURE, REVERSIBLE body-only projection for
// complete whole-block Markdown/Org code wrappers. `open + body + close ===`
// text always, so a body edit committed under the same wrapper preserves the
// exact raw wrapper bytes. Mixed content, incomplete/malformed wrappers, and
// ```calc (its own mode) are excluded — those shapes keep raw editing.
describe("codeBodyProjection", () => {
  it("splits a complete markdown fence into exact open/body/close bytes", () => {
    const text = "```js\nconst x = 1;\nconsole.log(x);\n```";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "```js\n", body: "const x = 1;\nconsole.log(x);", close: "\n```", lang: "js" });
    expect(p!.open + p!.body + p!.close).toBe(text);
  });

  it("keeps an empty scaffold's structural separator out of the payload", () => {
    const text = "```\n\n```";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "```\n", body: "", close: "\n```", lang: "" });
    expect(p!.open + p!.body + p!.close).toBe(text);
  });

  it("distinguishes a closer with no blank body line from the scaffold", () => {
    const text = "```js\n```";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "```js\n", body: "", close: "```", lang: "js" });
    expect(p!.open + p!.body + p!.close).toBe(text);
  });

  it("preserves exact wrapper bytes: tilde fence, longer runs, closer-only trailing blanks", () => {
    const text = "~~~sh\necho hi\n~~~\n\n";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "~~~sh\n", body: "echo hi", close: "\n~~~\n\n", lang: "sh" });
    expect(p!.open + p!.body + p!.close).toBe(text);
  });

  it("a shorter fence run is body content, not the closer (CommonMark length rule)", () => {
    const text = "````js\n```\nconst x = 1\n````";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "````js\n", body: "```\nconst x = 1", close: "\n````", lang: "js" });
    expect(p!.open + p!.body + p!.close).toBe(text);
    // And the same text without its closing four-run is INCOMPLETE (typing it
    // by hand must not make the shorter inner run the closer).
    expect(codeBodyProjection("````js\n```\nconst x = 1", "md")).toBeNull();
  });

  it("a fence run with an info string is body content, never a closer", () => {
    const text = "```\n```x\n```";
    const p = codeBodyProjection(text, "md");
    expect(p).toEqual({ open: "```\n", body: "```x", close: "\n```", lang: "" });
    expect(p!.open + p!.body + p!.close).toBe(text);
  });

  it("projects org #+begin_src and #+begin_example with exact case preserved", () => {
    const src = "#+BEGIN_SRC python\nx = 1\n#+END_SRC";
    const p = codeBodyProjection(src, "org");
    expect(p).toEqual({ open: "#+BEGIN_SRC python\n", body: "x = 1", close: "\n#+END_SRC", lang: "python" });
    expect(p!.open + p!.body + p!.close).toBe(src);
    const ex = "#+begin_example\nstuff\n#+end_example";
    const q = codeBodyProjection(ex, "org");
    expect(q).toEqual({ open: "#+begin_example\n", body: "stuff", close: "\n#+end_example", lang: "" });
    expect(q!.open + q!.body + q!.close).toBe(ex);
  });

  it("excludes calc, mixed, and incomplete wrappers so they keep raw editing", () => {
    expect(codeBodyProjection("```calc\n1 + 1\n```", "md")).toBeNull();
    expect(codeBodyProjection("intro\n```js\nx\n```", "md")).toBeNull();
    expect(codeBodyProjection("```js\nx\n```\ntailing note", "md")).toBeNull();
    expect(codeBodyProjection("```js\nx\n```\n```py\ny", "md")).toBeNull();
    expect(codeBodyProjection("```js\nx", "md")).toBeNull();
    expect(codeBodyProjection("#+BEGIN_SRC python\nx", "org")).toBeNull();
    expect(codeBodyProjection("plain text", "md")).toBeNull();
  });

  it("payload range for GH #412: the body sits between the exact wrapper bytes", () => {
    const text = "```\necho hello\n```";
    const p = codeBodyProjection(text, "md");
    expect(p!.body).toBe("echo hello");
    // Selecting the body in the projection's coordinate space excludes both fences.
    expect(text.slice(p!.open.length, p!.open.length + p!.body.length)).toBe("echo hello");
    expect(p!.body).not.toContain("```");
  });
});

// The join is the projection's inverse for edited bodies — crucially it keeps
// the closer's bytes intact when the user types into the phantom last row or
// the double-Enter exit trims the trailing blank line.
describe("codeBodyJoin", () => {
  const proj = { open: "```js\n", close: "\n```" };

  it("round-trips every projected body", () => {
    for (const body of ["", "\n", "const x = 1;\n", "const x = 1\n\n"]) {
      const raw = codeBodyJoin(proj, body);
      expect(codeBodyProjection(raw, "md")!.body).toBe(body);
    }
  });

  it("keeps the structural closer separator outside the editable body", () => {
    expect(codeBodyJoin(proj, "const x = 1\n()")).toBe("```js\nconst x = 1\n()\n```");
    // The double-Enter exit's trimmed body has no trailing newline.
    expect(codeBodyJoin(proj, "const x = 1")).toBe("```js\nconst x = 1\n```");
    // First character in an empty code block.
    expect(codeBodyJoin(proj, "x")).toBe("```js\nx\n```");
  });

  it("does not manufacture a payload newline across character-by-character commits", () => {
    let raw = codeBodyJoin(proj, "n");
    expect(codeBodyProjection(raw, "md")!.body).toBe("n");
    raw = codeBodyJoin(codeBodyProjection(raw, "md")!, "na");
    expect(codeBodyProjection(raw, "md")!.body).toBe("na");
  });
});

// The body-space counterpart of the fence double-Enter exit: with the caret on
// a TRAILING blank body line, Enter removes that sentinel line and leaves the
// block (the caller then creates the sibling). Blank lines in the middle are
// ordinary content; an all-blank body never exits.
describe("codeBodyExitTrim", () => {
  it("trims the trailing blank sentinel line when asked to exit", () => {
    const body = "const x = 1\n\n";
    // Caret on the sentinel blank line (the position Enter moves to after the
    // first press at the end of the body).
    const sentinel = body.indexOf("\n\n") + 1;
    expect(codeBodyExitTrim(body, sentinel)).toBe("const x = 1");
    // The same exit pressed at the very end of the body keeps the body's one
    // structural newline (the raw stays byte-identical to the no-blank form).
    expect(codeBodyExitTrim(body, body.length)).toBe("const x = 1\n");
  });

  it("returns null mid-body and on interior blank lines", () => {
    const body = "a\n\nb\n";
    expect(codeBodyExitTrim(body, 0)).toBeNull();
    expect(codeBodyExitTrim(body, 2)).toBeNull(); // interior blank line
    expect(codeBodyExitTrim("one line\n", 2)).toBeNull();
  });

  it("never exits an all-blank body", () => {
    expect(codeBodyExitTrim("\n", 0)).toBeNull();
    expect(codeBodyExitTrim("\n", 1)).toBeNull();
    expect(codeBodyExitTrim("\n\n\n", 3)).toBeNull();
  });
});
