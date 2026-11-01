import { afterAll, describe, expect, it } from "vitest";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { parsePluginManifest } from "./manifest";
import { pluginGrammar } from "./grammar.mjs";

const fixture = JSON.parse(readFileSync("fixtures/plugin-graph-ownership/manifest.json", "utf8"));
const schema = JSON.parse(readFileSync("plugin-sdk/schema/manifest.schema.json", "utf8"));
mkdirSync("target/artifacts/l702-pg", { recursive: true });
const directory = mkdtempSync("target/artifacts/l702-pg/grammar-");
copyFileSync("fixtures/plugin-graph-ownership/plugin.wasm", `${directory}/plugin.wasm`);
afterAll(() => rmSync(directory, { recursive: true, force: true }));

function acceptsSchema(rule: { pattern?: string; maxLength?: number; not?: { pattern: string } }, value: string) {
  return (!rule.pattern || new RegExp(rule.pattern).test(value)) &&
    (!rule.maxLength || value.length <= rule.maxLength) &&
    (!rule.not || !new RegExp(rule.not.pattern).test(value));
}

describe("PLUGIN-GRAMMAR host, checker and schema agreement", () => {
  it("keeps the schema definitions equal to the shared parser contract", () => {
    expect(schema.properties.id).toEqual(pluginGrammar.packageId);
    expect(schema.properties.entry).toEqual(pluginGrammar.entry);
    for (const kind of ["command", "slashCommand", "decoration"]) {
      expect(schema.$defs[kind].properties.id).toEqual(pluginGrammar.contributionId);
    }
  });
  it.each([
    ["dev.example", true], ["a..b", true], ["a.b-", false],
    ["a_b.c", false], ["abc", false], ["a", false], ["A.b", false],
    ["a." + "b".repeat(62), true], ["a." + "b".repeat(63), false], ["a.b\n", false],
  ])("keeps dotted package ID %s (%s)", (id, accepted) => {
    const manifest = { ...fixture, id };
    if (accepted) expect(parsePluginManifest(manifest).id).toBe(id);
    else expect(() => parsePluginManifest(manifest)).toThrow();
    writeFileSync(`${directory}/manifest.json`, JSON.stringify(manifest));
    const checked = spawnSync(process.execPath, ["scripts/tine-plugin.mjs", "check", directory, "--json"], { encoding: "utf8" });
    const report = JSON.parse(checked.stdout);
    expect(!report.errors.some((error: { code: string }) => error.code === "manifest.id")).toBe(accepted);
    expect(acceptsSchema(schema.properties.id, id)).toBe(accepted);
  });
  it.each(["commands", "slashCommands", "blockDecorations"])("checks %s IDs before packaging", (kind) => {
    const capability = { commands: "commands.register", slashCommands: "slash-commands.register", blockDecorations: "block-decorations.register" }[kind]!;
    for (const id of ["a", "hello", "a._-0", "a".repeat(64), "UPPER", "../bad", "", "a".repeat(65), "hello\n"]) {
      const contribution = kind === "blockDecorations" ? { id, kind: "badge" } : { id, title: "Example" };
      const manifest = { ...fixture, capabilities: [capability], contributions: { [kind]: [contribution] } };
      const accepted = /^[a-z0-9][a-z0-9._-]{0,63}$/.test(id) && !id.endsWith("\n");
      if (accepted) expect(() => parsePluginManifest(manifest)).not.toThrow();
      else expect(() => parsePluginManifest(manifest)).toThrow();
      writeFileSync(`${directory}/manifest.json`, JSON.stringify(manifest));
      const checked = spawnSync(process.execPath, ["scripts/tine-plugin.mjs", "check", directory, "--json"], { encoding: "utf8" });
      const report = JSON.parse(checked.stdout);
      expect(!report.errors.some((error: { code: string }) => error.code === "manifest.contribution-id"), `${kind}: ${JSON.stringify(id)}`).toBe(accepted);
      const definition = { commands: "command", slashCommands: "slashCommand", blockDecorations: "decoration" }[kind]!;
      expect(acceptsSchema(schema.$defs[definition].properties.id, id), `schema ${kind}: ${JSON.stringify(id)}`).toBe(accepted);
    }
  });

  it.each([
    ["plugin.wasm", true], ["dist/plugin.wasm", true], ["dist/./plugin.wasm", true],
    ["dist/plugin..wasm", true], ["dist/../plugin.wasm", false], ["../plugin.wasm", false],
    ["/plugin.wasm", false], ["plugin.WASM", false], ["plugin.wasm\n", false],
    ["dist//plugin.wasm", true], ["dist\\plugin.wasm", false], [`${"a".repeat(155)}.wasm`, true],
    [`${"a".repeat(156)}.wasm`, false],
  ])("checks entry %s (%s)", (entry, accepted) => {
    const manifest = { ...fixture, entry };
    if (accepted) expect(parsePluginManifest(manifest).entry).toBe(entry);
    else expect(() => parsePluginManifest(manifest)).toThrow();
    writeFileSync(`${directory}/manifest.json`, JSON.stringify(manifest));
    const checked = spawnSync(process.execPath, ["scripts/tine-plugin.mjs", "check", directory, "--json"], { encoding: "utf8" });
    const report = JSON.parse(checked.stdout);
    expect(!report.errors.some((error: { code: string }) => error.code === "manifest.entry-path")).toBe(accepted);
    expect(acceptsSchema(schema.properties.entry, entry)).toBe(accepted);
  });
});
