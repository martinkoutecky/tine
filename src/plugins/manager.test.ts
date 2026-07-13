import { describe, expect, it } from "vitest";
import { backend } from "../backend";
import { installedPlugins, PluginManager } from "./manager";

describe("installed plugin lifecycle", () => {
  it("uninstalls an incompatible stored manifest through its real package identity", async () => {
    const manifest = {
      schemaVersion: 1,
      id: "page.tine.legacy-uninstall-test",
      name: "Legacy uninstall test",
      version: "0.1.0",
      apiVersion: "0.1",
      description: "An intentionally incompatible installed package.",
      author: "Tine",
      license: "MIT",
      source: "https://example.invalid/legacy",
      entry: "plugin.wasm",
      platforms: ["desktop"],
      capabilities: [],
    };
    await backend().installPlugin(JSON.stringify(manifest), new Uint8Array([0, 97, 115, 109]));
    const manager = new PluginManager();
    await manager.initialize();
    const incompatible = installedPlugins().find((plugin) => plugin.error?.includes("apiVersion must be 0.2"));

    expect(incompatible?.manifest.id).toMatch(/^invalid\./);
    await expect(
      manager.uninstall(incompatible!.manifest.id, incompatible!.manifest.version)
    ).resolves.toBeUndefined();
    await manager.initialize();
    expect(installedPlugins().some((plugin) => plugin.error?.includes("apiVersion must be 0.2"))).toBe(false);
  });
});
