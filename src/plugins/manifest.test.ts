import { describe, expect, it } from "vitest";
import { parsePluginManifest, supportsPlatform } from "./manifest";

const base = {
  schemaVersion: 1,
  id: "dev.tine.example",
  name: "Example",
  version: "0.1.0",
  apiVersion: "0.2",
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

  it("accepts a remappable default binding for a command contribution", () => {
    const manifest = parsePluginManifest({
      ...base,
      contributions: { commands: [{ id: "hello", title: "Say hello", defaultBinding: "mod+1" }] },
    });
    expect(manifest.contributions?.commands?.[0].defaultBinding).toBe("mod+1");
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

  it("parses declarative settings and requires settings.read", () => {
    const settings = [{
      key: "active-only", type: "boolean", label: "Active only",
      description: "Show only the active ancestry.", default: false,
    }];
    expect(() => parsePluginManifest({ ...base, settings })).toThrow(/settings.read/);
    const manifest = parsePluginManifest({ ...base, capabilities: ["commands.register", "settings.read"], settings });
    expect(manifest.settings?.[0]).toMatchObject({ key: "active-only", default: false });
  });

  it("records immutable behavioral-port provenance", () => {
    const manifest = parsePluginManifest({
      ...base,
      portedFrom: {
        ecosystem: "logseq",
        name: "Example original",
        source: "https://github.com/example/original",
        revision: "0123456789abcdef",
        license: "MIT",
        authors: ["Original Author"],
        relationship: "behavioral-port",
      },
    });
    expect(manifest.portedFrom?.authors).toEqual(["Original Author"]);
  });
});
