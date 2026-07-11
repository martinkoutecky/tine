import { describe, expect, it } from "vitest";
import { parsePluginManifest, supportsPlatform } from "./manifest";

const base = {
  schemaVersion: 1,
  id: "dev.tine.example",
  name: "Example",
  version: "0.1.0",
  apiVersion: "0.1",
  description: "A test plugin",
  author: "Tine",
  license: "MIT",
  source: "https://example.invalid/source",
  entry: "dist/plugin.wasm",
  capabilities: ["commands.register"],
  contributions: { commands: [{ id: "hello", title: "Say hello" }] },
};

describe("parsePluginManifest", () => {
  it("defaults an omitted platform declaration to desktop only", () => {
    const manifest = parsePluginManifest(base);
    expect(manifest.platforms).toEqual(["desktop"]);
    expect(supportsPlatform(manifest, "desktop")).toBe(true);
    expect(supportsPlatform(manifest, "android")).toBe(false);
  });

  it("supports manifest and contribution platform narrowing", () => {
    const manifest = parsePluginManifest({
      ...base,
      platforms: ["desktop", "android"],
      contributions: {
        commands: [{ id: "hello", title: "Say hello", platforms: ["android"] }],
      },
    });
    const command = manifest.contributions?.commands?.[0];
    expect(supportsPlatform(manifest, "desktop", command?.platforms)).toBe(false);
    expect(supportsPlatform(manifest, "android", command?.platforms)).toBe(true);
  });

  it("rejects traversal and undeclared contribution authority", () => {
    expect(() => parsePluginManifest({ ...base, entry: "../plugin.wasm" })).toThrow(/relative/);
    expect(() => parsePluginManifest({ ...base, capabilities: [] })).toThrow(/commands.register/);
  });

  it("rejects unknown capabilities", () => {
    expect(() => parsePluginManifest({ ...base, capabilities: ["network.anywhere"] })).toThrow(/unsupported/);
  });

  it("rejects unknown fields instead of silently widening the format", () => {
    expect(() => parsePluginManifest({ ...base, postinstall: "run-me" })).toThrow(/unknown field postinstall/);
    expect(() =>
      parsePluginManifest({
        ...base,
        contributions: { commands: [{ id: "hello", title: "Say hello", script: "alert(1)" }] },
      })
    ).toThrow(/unknown field script/);
  });
});
