import { afterAll, describe, expect, it } from "vitest";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { SUPPORTED_PLUGIN_API_VERSIONS, parsePluginManifest } from "./manifest";

// The package checker answers "is this manifest valid" with the host's parser,
// then adds registry policy. So a manifest the checker passes is one the app's
// manifest parser accepts; the installer still checks the platform and the stored
// size on its own. Each row changes one field of the shipped fixture.
const fixture = JSON.parse(readFileSync("fixtures/plugin-graph-ownership/manifest.json", "utf8"));
mkdirSync("target/artifacts/plugin-checker", { recursive: true });
const directory = mkdtempSync("target/artifacts/plugin-checker/agreement-");
copyFileSync("fixtures/plugin-graph-ownership/plugin.wasm", `${directory}/plugin.wasm`);
afterAll(() => rmSync(directory, { recursive: true, force: true }));

const command = fixture.contributions.commands[0];
const long = (n: number) => "x".repeat(n);
const color = { key: "tint", label: "Tint", description: "Badge tint", type: "color", default: "" };
// Each row: name, the one-field change, and the report code (`-` when the
// package passes). The code is the per-field code of tine-plugin-check/v1 for the
// field the parser refused, as the previous checker reported it wherever it
// refused the row, except "color setting on API 0.2" (it said
// `manifest.settings-color-api`; the refused field is the type).
type Row = [string, (m: Record<string, any>) => void, string];
const rows: Row[] = [
  ["unchanged", () => {}, "-"],
  ["81-character name", (m) => { m.name = long(81); }, "manifest.name"],
  ["80-character name", (m) => { m.name = long(80); }, "-"],
  ["duplicate capability", (m) => { m.capabilities = [...m.capabilities, "commands.register"]; }, "manifest.capabilities"],
  ["duplicate platform", (m) => { m.platforms = ["desktop", "desktop"]; }, "manifest.platforms"],
  ["empty platforms", (m) => { m.platforms = []; }, "manifest.platforms"],
  ["unknown platform", (m) => { m.platforms = ["desktop", "web"]; }, "manifest.platforms"],
  ["duplicate command", (m) => { m.contributions.commands = [command, command]; }, "manifest.contribution-id"],
  ["unknown capability", (m) => { m.capabilities = [...m.capabilities, "shell.run"]; }, "manifest.capabilities"],
  ["capabilities omitted", (m) => { delete m.capabilities; }, "manifest.capabilities"],
  ["contribution without its capability", (m) => { m.capabilities = ["graph.write.block"]; }, "manifest.contribution-capability"],
  ["schemaVersion 2", (m) => { m.schemaVersion = 2; }, "manifest.schema"],
  ["apiVersion 0.4", (m) => { m.apiVersion = "0.4"; }, "manifest.api"],
  ["65-character id", (m) => { m.id = `a.${long(63)}`; }, "manifest.id"],
  ["65-character version", (m) => { m.version = `1.0.0-${long(59)}`; }, "manifest.version"],
  ["501-character description", (m) => { m.description = long(501); }, "manifest.description"],
  ["161-character author", (m) => { m.author = long(161); }, "manifest.author"],
  ["unrecognized licence", (m) => { m.license = "Proprietary"; }, "manifest.license"],
  ["http source", (m) => { m.source = "http://example.com/plugin"; }, "manifest.source"],
  ["501-character source", (m) => { m.source = `https://example.com/${long(481)}`; }, "manifest.source"],
  ["traversing entry", (m) => { m.entry = "../plugin.wasm"; }, "manifest.entry-path"],
  ["unknown field", (m) => { m.homepage = "https://example.com"; }, "manifest.field"],
  ["aiDevelopment invalid", (m) => { m.aiDevelopment = "some"; }, "manifest.ai"],
  ["aiDevelopment omitted", (m) => { delete m.aiDevelopment; }, "-"],
  ["81-character command title", (m) => { m.contributions.commands = [{ ...command, title: long(81) }]; }, "manifest.invalid"],
  ["241-character command description", (m) => { m.contributions.commands = [{ ...command, description: long(241) }]; }, "manifest.invalid"],
  ["81-character binding", (m) => { m.contributions.commands = [{ ...command, defaultBinding: long(81) }]; }, "manifest.command-binding"],
  ["control character in binding", (m) => { m.contributions.commands = [{ ...command, defaultBinding: "mod+\u0007" }]; }, "manifest.command-binding"],
  ["duplicate command platform", (m) => { m.contributions.commands = [{ ...command, platforms: ["desktop", "desktop"] }]; }, "manifest.invalid"],
  ["unknown contribution kind", (m) => { m.contributions.panels = []; }, "manifest.contributions-field"],
  ["unknown decoration kind", (m) => {
    m.capabilities = [...m.capabilities, "block-decorations.register"];
    m.contributions.blockDecorations = [{ id: "dot", kind: "glow" }];
  }, "manifest.invalid"],
  ["4097-character slash insert", (m) => {
    m.capabilities = [...m.capabilities, "slash-commands.register"];
    m.contributions.slashCommands = [{ id: "hi", title: "Hi", insertText: long(4097) }];
  }, "manifest.invalid"],
  ["settings without settings.read", (m) => {
    m.settings = [{ key: "on", label: "On", description: "Turn on", type: "boolean", default: true }];
  }, "manifest.settings-capability"],
  ["duplicate setting key", (m) => {
    const setting = { key: "on", label: "On", description: "Turn on", type: "boolean", default: true };
    m.capabilities = [...m.capabilities, "settings.read"];
    m.settings = [setting, setting];
  }, "manifest.settings-key"],
  ["color setting on API 0.2", (m) => {
    m.capabilities = [...m.capabilities, "settings.read"];
    m.settings = [color];
  }, "manifest.settings-type"],
  ["color setting on API 0.3", (m) => {
    m.apiVersion = "0.3";
    m.capabilities = [...m.capabilities, "settings.read"];
    m.settings = [color];
  }, "-"],
  ["33 ported-from authors", (m) => {
    m.portedFrom = {
      ecosystem: "logseq", name: "x", source: "https://example.com/x", revision: "1", license: "MIT",
      authors: Array.from({ length: 33 }, (_, i) => `a${i}`), relationship: "behavioral-port",
    };
  }, "manifest.provenance"],
  ["http ported-from source", (m) => {
    m.portedFrom = {
      ecosystem: "logseq", name: "x", source: "http://example.com/x", revision: "1", license: "MIT",
      authors: ["a"], relationship: "behavioral-port",
    };
  }, "manifest.provenance"],
];

function check(manifest: unknown) {
  writeFileSync(`${directory}/manifest.json`, JSON.stringify(manifest));
  const run = spawnSync(process.execPath, ["scripts/tine-plugin.mjs", "check", directory, "--json"], { encoding: "utf8" });
  return JSON.parse(run.stdout) as { status: string; errors: Array<{ code: string; message: string }> };
}

function hostAccepts(manifest: unknown) {
  try {
    parsePluginManifest(manifest);
    return true;
  } catch {
    return false;
  }
}

describe("the package checker never passes a manifest the host refuses", () => {
  it.each(rows)("%s", (_name, mutate, code) => {
    const manifest = structuredClone(fixture);
    mutate(manifest);
    const report = check(manifest);
    if (report.status === "passed") expect(hostAccepts(manifest), JSON.stringify(report.errors)).toBe(true);
    expect(report.errors.map((error) => error.code).join(",") || "-").toBe(code);
  });

  it.each([
    ["81-character name", (m: Record<string, any>) => { m.name = long(81); }],
    ["duplicate capability", (m: Record<string, any>) => { m.capabilities = [...m.capabilities, "commands.register"]; }],
    ["duplicate platform", (m: Record<string, any>) => { m.platforms = ["desktop", "desktop"]; }],
  ])("refuses the %s the host refuses", (_name, mutate) => {
    const manifest = structuredClone(fixture);
    mutate(manifest);
    expect(hostAccepts(manifest)).toBe(false);
    expect(check(manifest).status).toBe("failed");
  });

  it("accepts every API version the host runs and refuses the next", () => {
    for (const apiVersion of SUPPORTED_PLUGIN_API_VERSIONS) {
      expect(check({ ...fixture, apiVersion }).status, apiVersion).toBe("passed");
    }
    expect(check({ ...fixture, apiVersion: "9.9" }).status).toBe("failed");
  });
});
