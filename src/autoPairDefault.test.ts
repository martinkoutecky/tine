// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

// Master 46cd3997c (GH #291): editor auto-pairing is ON by default (OG parity:
// Logseq always auto-pairs), and an explicit opt-out persists across reloads.
async function freshUi() {
  vi.resetModules();
  return import("./ui");
}

afterEach(() => {
  localStorage.clear();
  vi.resetModules();
});

describe("auto-pairing default (GH #291)", () => {
  it("is on for a device that never chose", async () => {
    localStorage.clear();
    expect((await freshUi()).autoPairing()).toBe(true);
  });

  it("keeps an explicit opt-out across a reload, and an opt-in back on", async () => {
    localStorage.clear();
    (await freshUi()).setAutoPairing(false);
    expect((await freshUi()).autoPairing()).toBe(false);
    (await freshUi()).setAutoPairing(true);
    expect((await freshUi()).autoPairing()).toBe(true);
  });
});
