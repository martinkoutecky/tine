import { describe, it, expect } from "vitest";
import {
  caretInFence,
  multilineExitTrim,
  isSheetCellHidden,
  joinProps,
  readPropertyValue,
  splitProps,
  upsertPropertyLine,
  isBuiltinHidden,
  rawOffsetToVisibleOffset,
  isPageHeaderPropertiesOnly,
  parsePageHeaderPropertyLine,
  splitPagePreamble,
} from "./properties";

describe("canonical Markdown page-header grammar (GH #163)", () => {
  it("accepts Unicode/plugin keys and preserves internal blank separators", () => {
    expect(parsePageHeaderPropertyLine("klíč:: hodnota")).toEqual({ key: "klíč", value: " hodnota" });
    expect(parsePageHeaderPropertyLine("e\u0301/plugin.key::value")).toEqual({ key: "e\u0301/plugin.key", value: "value" });
    expect(isPageHeaderPropertiesOnly("alias:: book\n\nklíč:: hodnota")).toBe(true);
    expect(splitPagePreamble("alias:: book\n\nklíč:: hodnota\n\nIntro")).toEqual({
      properties: "alias:: book\n\nklíč:: hodnota",
      content: "Intro",
      remainder: "\n\nIntro",
    });
  });

  it("rejects leading whitespace, headings, prose, fences and trailing blanks", () => {
    for (const raw of [" alias:: x", "#alias:: x", "alias key:: x", "```\na:: b\n```", "alias:: x\nprose", "alias:: x\n"]) {
      expect(isPageHeaderPropertiesOnly(raw), raw).toBe(false);
    }
  });
});

describe("multilineExitTrim — Enter on a trailing blank line exits a code/calc block", () => {
  it("fenced: exits only on the blank last content line and keeps the closing fence", () => {
    expect(multilineExitTrim("```js\ncode\n\n```", 11, "fence")).toBe("```js\ncode\n```");
    expect(multilineExitTrim("```js\na\n\nb\n```", 8, "fence")).toBeNull();
    expect(multilineExitTrim("```js\ncode\n```", 8, "fence")).toBeNull();
  });

  it("calc: exits only on a trailing blank line", () => {
    expect(multilineExitTrim("1+1\n2+2\n", 8, "calc")).toBe("1+1\n2+2");
    expect(multilineExitTrim("1+1\n\n2+2", 4, "calc")).toBeNull();
  });

  it("never exits from the first blank line", () => {
    expect(multilineExitTrim("\ncode", 0, "calc")).toBeNull();
    expect(multilineExitTrim("\n```", 0, "fence")).toBeNull();
  });
});

describe("sheet-cell property splitting", () => {
  it("hides built-in and tine properties while preserving them byte-exactly on join", () => {
    const raw = "Body line\ntine.view:: grid\nid:: abc-123";

    const split = splitProps(raw, isSheetCellHidden);

    expect(split.visible).toBe("Body line");
    expect(split.hidden).toBe("tine.view:: grid\nid:: abc-123");
    expect(joinProps("Changed body", split.hidden)).toBe("Changed body\ntine.view:: grid\nid:: abc-123");
  });
});

describe("property line helpers", () => {
  it("reads a value case-insensitively", () => {
    expect(readPropertyValue("alias:: Foo, Bar\npublic:: true", "alias")).toBe("Foo, Bar");
    expect(readPropertyValue("Alias:: Foo", "alias")).toBe("Foo");
    expect(readPropertyValue("public:: true", "missing")).toBe(null);
    expect(readPropertyValue(null, "alias")).toBe(null);
  });

  it("adds a new property", () => {
    expect(upsertPropertyLine(null, "alias", "Foo")).toBe("alias:: Foo");
    expect(upsertPropertyLine("tags:: x", "alias", "Foo")).toBe("alias:: Foo\ntags:: x");
  });

  it("replaces an existing property in place and preserves siblings", () => {
    expect(upsertPropertyLine("alias:: Old\ntags:: x", "alias", "New")).toBe("alias:: New\ntags:: x");
  });

  it("removes a property when value is null/empty, returning null if none remain", () => {
    expect(upsertPropertyLine("alias:: Foo", "alias", null)).toBe(null);
    expect(upsertPropertyLine("alias:: Foo", "alias", "  ")).toBe(null);
    expect(upsertPropertyLine("alias:: Foo\ntags:: x", "alias", null)).toBe("tags:: x");
  });

  it("trims the value while preserving blank separators", () => {
    expect(upsertPropertyLine("\n\ntags:: x\n\n", "alias", "  Foo  ")).toBe("alias:: Foo\n\n\ntags:: x\n\n");
  });

  it("preserves the issue-163 page-property layout byte-for-byte outside the edited line", () => {
    const before = [
      "alias:: Test Record",
      "ai-prompt:: [[Prompt-Test]]",
      "usage-frequency:: [[Frequency-High]]",
      "",
      "page-level:: [[Level-Two]]",
      "layout:: [[Layout-Top-Collapsed]]",
      "component-state:: [[Component-Wide]]",
      "",
      "timestamp:: 20250707092601",
      "observation-target:: [[Object-Test-Page]]",
      "external-impact::",
      "--:: --",
      "methods:: [[Method-A]] [[Method-B]]",
      "key-conclusion:: [[Conclusion-A]] [[Conclusion-B]]",
    ].join("\n");
    const expected = before.replace("alias:: Test Record", "alias:: Updated Record");

    expect(upsertPropertyLine(before, "alias", "Updated Record")).toBe(expected);
  });
});

describe("caretInFence", () => {
  it("is false before the opening fence", () => {
    const raw = "before\n```ts\nconst x = 1;\n```\nafter";
    expect(caretInFence(raw, raw.indexOf("before"))).toBe(false);
  });

  it("is true on a line inside the fence", () => {
    const raw = "before\n```ts\nconst x = 1;\n```\nafter";
    expect(caretInFence(raw, raw.indexOf("const"))).toBe(true);
  });

  it("is false after the closing fence", () => {
    const raw = "before\n```ts\nconst x = 1;\n```\nafter";
    expect(caretInFence(raw, raw.indexOf("after"))).toBe(false);
  });

  it("is true inside an unterminated fence", () => {
    const raw = "before\n```\nconst x = 1;";
    expect(caretInFence(raw, raw.indexOf("const"))).toBe(true);
  });

  it("does not close a four-character fence with a shorter run", () => {
    const raw = "````text\nalpha\n```\nid:: literal-code\n````\nid:: real-id";
    expect(caretInFence(raw, raw.indexOf("literal-code"))).toBe(true);
    const { visible, hidden } = splitProps(raw, isBuiltinHidden);
    expect(visible).toContain("id:: literal-code");
    expect(hidden).toBe("id:: real-id");
  });

  it("uses the opening run length for tilde fences too", () => {
    const raw = "~~~~text\n~~~\ncollapsed:: literal-code\n~~~~\ncollapsed:: true";
    expect(caretInFence(raw, raw.indexOf("literal-code"))).toBe(true);
    const { visible, hidden } = splitProps(raw, isBuiltinHidden);
    expect(visible).toContain("collapsed:: literal-code");
    expect(hidden).toBe("collapsed:: true");
  });
});

// GH #37: org block-property drawers (`:PROPERTIES:`/`:id:`/`:END:`) must be
// hidden from the editor like markdown `id::`, and round-trip losslessly.
describe("org :PROPERTIES: drawer hiding (GH #37)", () => {
  it("hides an id-only drawer entirely (wrapper + line)", () => {
    const raw = "TODO Task\n:PROPERTIES:\n:id: abc-123\n:END:";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe("TODO Task");
    expect(hidden).toBe(":id: abc-123");
  });

  it("round-trips an id-only drawer back to canonical position", () => {
    const raw = "TODO Task\n:PROPERTIES:\n:id: abc-123\n:END:";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(joinProps(visible, hidden, "org")).toBe(raw);
  });

  it("keeps the drawer + user props visible, hiding only the built-in id line", () => {
    const raw = "Title\n:PROPERTIES:\n:owner: martin\n:id: u1\n:END:\nbody";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe("Title\n:PROPERTIES:\n:owner: martin\n:END:\nbody");
    expect(hidden).toBe(":id: u1");
    expect(joinProps(visible, hidden, "org")).toBe(
      "Title\n:PROPERTIES:\n:owner: martin\n:id: u1\n:END:\nbody"
    );
  });

  it("groups a freshly reattached drawer after SCHEDULED/DEADLINE planning lines", () => {
    const raw = "TODO Task\nSCHEDULED: <2026-07-07 Tue>\n:PROPERTIES:\n:id: x\n:END:\nbody";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe("TODO Task\nSCHEDULED: <2026-07-07 Tue>\nbody");
    expect(joinProps(visible, hidden, "org")).toBe(raw);
  });

  it("leaves a block with no drawer untouched", () => {
    const raw = "just some text\nsecond line";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe(raw);
    expect(hidden).toBe("");
    expect(joinProps(visible, hidden, "org")).toBe(raw);
  });

  it("does NOT treat a stray md `key::` line in an org block as metadata", () => {
    const raw = "Title\ncollapsed:: true";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe(raw);
    expect(hidden).toBe("");
  });

  it("leaves a :LOGBOOK: drawer visible (not a property drawer)", () => {
    const raw = "DONE Ship\n:LOGBOOK:\nCLOCK: [2026-07-07 Tue 09:00]\n:END:";
    const { visible } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible).toBe(raw);
  });
});

describe("markdown property hiding is unchanged (regression)", () => {
  it("still hides md id:: and appends on reattach", () => {
    const raw = "Task\nid:: abc";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden);
    expect(visible).toBe("Task");
    expect(hidden).toBe("id:: abc");
    expect(joinProps(visible, hidden)).toBe(raw);
  });

  it("md default does not touch a `:PROPERTIES:` block (treated as content)", () => {
    const raw = "Task\n:PROPERTIES:\n:id: x\n:END:";
    const { visible } = splitProps(raw, isBuiltinHidden); // format defaults to md
    expect(visible).toBe(raw);
  });
});

describe("org caret mapping across a hidden drawer", () => {
  it("maps a raw offset in the body below a hidden drawer into visible space", () => {
    const raw = "Title\n:PROPERTIES:\n:id: x\n:END:\nbody";
    const rawOff = raw.indexOf("body");
    const visOff = rawOffsetToVisibleOffset(raw, rawOff, isBuiltinHidden, "org");
    const { visible } = splitProps(raw, isBuiltinHidden, "org");
    expect(visible.slice(visOff)).toBe("body");
  });

  it("maps an offset inside the hidden drawer to the drawer's edit point", () => {
    const raw = "Title\n:PROPERTIES:\n:id: x\n:END:\nbody";
    const insideDrawer = raw.indexOf(":id:") + 2;
    const visOff = rawOffsetToVisibleOffset(raw, insideDrawer, isBuiltinHidden, "org");
    expect(visOff).toBe("Title".length);
  });
});
