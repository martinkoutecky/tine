import { describe, it, expect } from "vitest";
import {
  caretInFence,
  isSheetCellHidden,
  joinProps,
  readPropertyValue,
  splitProps,
  upsertPropertyLine,
} from "./properties";

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
    expect(upsertPropertyLine("tags:: x", "alias", "Foo")).toBe("tags:: x\nalias:: Foo");
  });

  it("replaces an existing property in place-ish and preserves siblings", () => {
    expect(upsertPropertyLine("alias:: Old\ntags:: x", "alias", "New")).toBe("tags:: x\nalias:: New");
  });

  it("removes a property when value is null/empty, returning null if none remain", () => {
    expect(upsertPropertyLine("alias:: Foo", "alias", null)).toBe(null);
    expect(upsertPropertyLine("alias:: Foo", "alias", "  ")).toBe(null);
    expect(upsertPropertyLine("alias:: Foo\ntags:: x", "alias", null)).toBe("tags:: x");
  });

  it("trims the value and drops blank lines", () => {
    expect(upsertPropertyLine("\n\ntags:: x\n\n", "alias", "  Foo  ")).toBe("tags:: x\nalias:: Foo");
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
});
