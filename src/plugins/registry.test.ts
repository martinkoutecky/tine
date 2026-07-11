import { describe, expect, it } from "vitest";
import { parseRegistryIndex } from "./registry";

const version = {
  version: "0.1.0",
  apiVersion: "0.1",
  platforms: ["desktop"],
  capabilities: [],
  sha256: "a".repeat(64),
  manifestSha256: "b".repeat(64),
  manifestUrl: "https://example.invalid/manifest.json",
  wasmUrl: "https://example.invalid/plugin.wasm",
  audit: { status: "passed", url: "https://example.invalid/audit.json" },
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

describe("signed plugin registry parsing", () => {
  it("accepts a bounded HTTPS catalogue after the native signature boundary", () => {
    const parsed = parseRegistryIndex({
      schemaVersion: 1,
      generatedAt: "2026-07-11T00:00:00Z",
      plugins: [plugin],
      revocations: [],
    });
    expect(parsed.plugins[0].versions[0].sha256).toBe("a".repeat(64));
  });

  it("rejects duplicate identities, invalid digests, and non-HTTPS artifacts", () => {
    const root = { schemaVersion: 1, generatedAt: "now", plugins: [plugin, plugin], revocations: [] };
    expect(() => parseRegistryIndex(root)).toThrow(/duplicate/);
    expect(() =>
      parseRegistryIndex({ ...root, plugins: [{ ...plugin, versions: [{ ...version, sha256: "bad" }] }] })
    ).toThrow(/digest/);
    expect(() =>
      parseRegistryIndex({ ...root, plugins: [{ ...plugin, versions: [{ ...version, wasmUrl: "http://unsafe/plugin.wasm" }] }] })
    ).toThrow(/https/);
  });
});
