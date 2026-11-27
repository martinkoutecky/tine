// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

// (reverses the GH #583 default): a device that never chose sees
// arrows and dashes as glyphs while the file keeps the ASCII; "off" and "type"
// are explicit choices that persist across reloads.
async function freshUi() {
  vi.resetModules();
  return import("./ui");
}

afterEach(() => {
  localStorage.clear();
  vi.resetModules();
});

describe("typographic replacements default", () => {
  it("is on while reading for a device that never chose", async () => {
    localStorage.clear();
    expect((await freshUi()).typographyMode()).toBe("render");
  });

  it("keeps an explicit opt-out or the typing mode across a reload, and the default back on", async () => {
    localStorage.clear();
    (await freshUi()).setTypographyMode("off");
    expect((await freshUi()).typographyMode()).toBe("off");
    (await freshUi()).setTypographyMode("type");
    expect((await freshUi()).typographyMode()).toBe("type");
    (await freshUi()).setTypographyMode("render");
    expect((await freshUi()).typographyMode()).toBe("render");
  });

  it("reads a value stored by an earlier version the same way", async () => {
    // Earlier versions stored "render" and "type"; their "off" was the absent key.
    localStorage.setItem("logseq-claude.typography", "render");
    expect((await freshUi()).typographyMode()).toBe("render");
  });
});
