import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { parsePluginManifest } from "./manifest";
import { parseRegistryIndex } from "./registry";

const fixtureRoot = path.resolve("fixtures/plugin-revocation");
const read = (name: string) => fs.readFileSync(path.join(fixtureRoot, name));
const json = (name: string) => JSON.parse(read(name).toString("utf8"));
const sha256 = (bytes: Uint8Array) => crypto.createHash("sha256").update(bytes).digest("hex");
const canonical = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, child]) => [key, canonical(child)]));
  }
  return value;
};

describe("production revocation E2E fixture preparation", () => {
  it("keeps a reserved harmless manifest and valid deterministic guest", () => {
    const manifest = parsePluginManifest(json("manifest.json"));
    expect(`${manifest.id}@${manifest.version}`).toBe("page.tine.e2e.revocation-sentinel@0.0.1");
    expect(manifest.capabilities).toEqual(["block-decorations.register"]);
    expect(manifest.contributions?.blockDecorations).toEqual([
      { id: "revocation-sentinel-lines", kind: "thread-lines" },
    ]);

    const module = new WebAssembly.Module(read("plugin.wasm"));
    expect(WebAssembly.Module.imports(module)).toEqual([{ module: "env", name: "memory", kind: "memory" }]);
    expect(WebAssembly.Module.exports(module).map(({ name }) => name).sort()).toEqual([
      "tine_alloc",
      "tine_handle",
      "tine_result_len",
    ]);
  });

  it("binds the exact manifest, guest, cache bytes, and canonical revoked identity", () => {
    const metadata = json("fixture.json");
    expect(metadata).toMatchObject({
      schemaVersion: 1,
      identity: "page.tine.e2e.revocation-sentinel@0.0.1",
      manifestSha256: sha256(read("manifest.json")),
      wasmSha256: sha256(read("plugin.wasm")),
      controlIndexSha256: sha256(read("control-index.json")),
      revokedIndexSha256: sha256(read("revoked-index.json")),
      publicKeySha256: sha256(read("registry-ed25519.pub.pem")),
    });
    const revoked = parseRegistryIndex(json("revoked-index.json"));
    expect(revoked.revocations).toEqual([
      expect.objectContaining({ id: "page.tine.e2e.revocation-sentinel", version: "0.0.1" }),
    ]);
    expect(read("revoked-index.json").toString("utf8")).toBe(`${JSON.stringify(canonical(revoked), null, 2)}\n`);
  });

  it("verifies the public positive-control signature without treating it as revocation proof", () => {
    const valid = crypto.verify(
      null,
      read("control-index.json"),
      read("registry-ed25519.pub.pem"),
      Buffer.from(read("control-index.json.sig").toString("utf8").trim(), "base64"),
    );
    expect(valid).toBe(true);
    expect(parseRegistryIndex(json("control-index.json")).revocations).toEqual([]);
  });
});
