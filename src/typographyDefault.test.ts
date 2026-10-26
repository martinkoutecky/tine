// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

// GH #583: text renders as typed (`->` stays `->`, as in Logseq) for a device
// that never chose; the glyph modes are opt-in and persist across reloads.
async function freshUi() {
  vi.resetModules();
  return import("./ui");
}

afterEach(() => {
  localStorage.clear();
  vi.resetModules();
});

describe("typographic replacements default (GH #583)", () => {
  it("is off for a device that never chose", async () => {
    localStorage.clear();
    expect((await freshUi()).typographyMode()).toBe("off");
  });

  it("keeps an explicit opt-in to either glyph mode across a reload, and an opt-out back off", async () => {
    localStorage.clear();
    (await freshUi()).setTypographyMode("render");
    expect((await freshUi()).typographyMode()).toBe("render");
    (await freshUi()).setTypographyMode("type");
    expect((await freshUi()).typographyMode()).toBe("type");
    (await freshUi()).setTypographyMode("off");
    expect((await freshUi()).typographyMode()).toBe("off");
  });
});
