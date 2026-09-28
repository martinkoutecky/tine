import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { toasts, setToasts } from "../toasts";
import { loadSafetyReport, type RegistryPlugin, type RegistryVersion } from "./registry";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); setToasts([]); });

it("reports a failed safety-report cache write", async () => {
  const report = {
    format: "tine-plugin-audit-result/v1",
    submission: { pluginId: "dev.tine.example", version: "0.1.0", commit: "d".repeat(40) },
    commitVerified: "d".repeat(40),
    disposition: "quarantine",
    checker: { status: "passed", risk: "review", checkedAt: "2026-07-11T00:00:00Z" },
    aiReview: { disposition: "pass", uncertain: false, summary: "Reviewed.", findings: [], areasReviewed: ["graph effects"] },
    manualApproval: { by: "Sol", note: "Reviewed.", approvedAt: "2026-07-11T00:10:00Z" },
  };
  const bytes = new TextEncoder().encode(JSON.stringify(report));
  const sha256 = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
    .map((byte) => byte.toString(16).padStart(2, "0")).join("");
  const version = {
    version: "0.1.0", audit: { sha256, url: "https://example.invalid/audit.json", status: "passed", risk: "review", automatedDisposition: "quarantine", manualApproval: true, checkedAt: "2026-07-11T00:00:00Z" },
  } as RegistryVersion;
  const plugin = { id: "dev.tine.example", versions: [version] } as RegistryPlugin;
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(bytes)));
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
  await loadSafetyReport(plugin, version);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
});
