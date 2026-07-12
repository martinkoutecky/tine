import { describe, expect, it } from "vitest";
import { parseRegistryIndex, parseSafetyReport } from "./registry";

const version = {
  version: "0.1.0",
  apiVersion: "0.2",
  platforms: ["desktop"],
  capabilities: [],
  sha256: "a".repeat(64),
  manifestSha256: "b".repeat(64),
  manifestUrl: "https://example.invalid/manifest.json",
  wasmUrl: "https://example.invalid/plugin.wasm",
  audit: {
    status: "passed",
    url: "https://example.invalid/audit.json",
    sha256: "c".repeat(64),
    risk: "review",
    automatedDisposition: "quarantine",
    manualApproval: true,
    checkedAt: "2026-07-11T00:00:00Z",
  },
  publishedAt: "2026-07-11T00:00:00Z",
};
const plugin = {
  id: "dev.tine.example",
  name: "Example",
  description: "Example plugin",
  source: "https://example.invalid/source",
  license: "MIT",
  aiDevelopment: "primary",
  versions: [version],
};
const theme = {
  id: "page.tine.theme.example",
  name: "Example theme",
  description: "An inert token theme",
  source: "https://example.invalid/theme",
  license: "MIT",
  aiDevelopment: "primary",
  versions: [{
    version: "1.0.0",
    apiVersion: "0.1",
    modes: ["light", "dark"],
    manifestSha256: "e".repeat(64),
    manifestUrl: "https://example.invalid/theme.json",
    audit: { ...version.audit, risk: "low", automatedDisposition: "publish", manualApproval: false },
    publishedAt: "2026-07-12T00:00:00Z",
  }],
};

describe("signed plugin registry parsing", () => {
  it("accepts a bounded HTTPS catalogue after the native signature boundary", () => {
    const parsed = parseRegistryIndex({
      schemaVersion: 1,
      generatedAt: "2026-07-11T00:00:00Z",
      plugins: [plugin],
      themes: [theme],
      revocations: [],
    });
    expect(parsed.plugins[0].versions[0].sha256).toBe("a".repeat(64));
    expect(parsed.themes[0].versions[0].modes).toEqual(["light", "dark"]);
  });

  it("rejects duplicate identities, invalid digests, and non-HTTPS artifacts", () => {
    const root = { schemaVersion: 1, generatedAt: "now", plugins: [plugin, plugin], themes: [], revocations: [] };
    expect(() => parseRegistryIndex(root)).toThrow(/duplicate/);
    expect(() =>
      parseRegistryIndex({ ...root, plugins: [{ ...plugin, versions: [{ ...version, sha256: "bad" }] }] })
    ).toThrow(/digest/);
    expect(() =>
      parseRegistryIndex({ ...root, plugins: [{ ...plugin, versions: [{ ...version, wasmUrl: "http://unsafe/plugin.wasm" }] }] })
    ).toThrow(/https/);
  });

  it("parses a digest-bound manual safety review and rejects summary drift", () => {
    const parsed = parseRegistryIndex({
      schemaVersion: 1,
      generatedAt: "2026-07-11T00:00:00Z",
      plugins: [plugin],
      themes: [],
      revocations: [],
    });
    const registeredPlugin = parsed.plugins[0];
    const registeredVersion = registeredPlugin.versions[0];
    const report = {
      format: "tine-plugin-audit-result/v1",
      submission: { pluginId: plugin.id, version: version.version, commit: "d".repeat(40) },
      commitVerified: "d".repeat(40),
      disposition: "quarantine",
      checker: { status: "passed", risk: "review", checkedAt: "2026-07-11T00:00:00Z" },
      aiReview: {
        disposition: "pass",
        uncertain: false,
        summary: "A focused write was reviewed.",
        findings: [{ severity: "low", title: "Focused write", impact: "Only the focused block can change." }],
        areasReviewed: ["graph effects"],
      },
      manualApproval: { by: "Sol", note: "Reviewed after a scope fix.", approvedAt: "2026-07-11T00:10:00Z" },
    };
    expect(parseSafetyReport(report, registeredPlugin, registeredVersion).manualApproval?.note).toMatch(/scope fix/);
    expect(() =>
      parseSafetyReport({ ...report, checker: { ...report.checker, risk: "low" } }, registeredPlugin, registeredVersion)
    ).toThrow(/signed summary/);
  });
});
