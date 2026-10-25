import { afterEach, expect, it, vi } from "vitest";
import { cm6SpikeEnabled } from "./cm6Spike";

afterEach(() => { vi.unstubAllEnvs(); vi.unstubAllGlobals(); });

function environment(build: boolean, search = "", choice: string | null = null) {
  vi.stubEnv("MODE", "production");
  vi.stubEnv("DEV", false);
  vi.stubEnv("VITE_CM6_SPIKE", build ? "1" : "");
  vi.stubGlobal("location", { search });
  vi.stubGlobal("sessionStorage", { getItem: () => choice });
}

it("mounts CM6 without a console opt-in in a spike build", () => {
  environment(true);
  expect(cm6SpikeEnabled()).toBe(true);
});

it("keeps ordinary production builds off even with a runtime opt-in", () => {
  environment(false, "?cm6=1", "1");
  expect(cm6SpikeEnabled()).toBe(false);
});

it("allows same-build textarea comparison and query precedence", () => {
  environment(true, "", "0");
  expect(cm6SpikeEnabled()).toBe(false);
  environment(true, "?cm6=1", "0");
  expect(cm6SpikeEnabled()).toBe(true);
  environment(true, "?cm6=0", "1");
  expect(cm6SpikeEnabled()).toBe(false);
});

it("keeps all test-mode builds off, including accidental spike environments", () => {
  environment(true, "?cm6=1", "1");
  vi.stubEnv("MODE", "test");
  expect(cm6SpikeEnabled()).toBe(false);
});

it("retains explicit dev opt-in without enabling ordinary dev sessions", () => {
  environment(false);
  vi.stubEnv("DEV", true);
  expect(cm6SpikeEnabled()).toBe(false);
  environment(false, "?cm6=1");
  vi.stubEnv("DEV", true);
  expect(cm6SpikeEnabled()).toBe(true);
});
