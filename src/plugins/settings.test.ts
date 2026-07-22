import { describe, expect, it } from "vitest";
import {
  defaultPluginSettings,
  parsePluginSettingDefinitions,
  parsePluginSettingsBlob,
  settingAccepts,
  validatePluginSettings,
} from "./settings";

const definitions = parsePluginSettingDefinitions([
  { key: "active-only", type: "boolean", label: "Active only", description: "Show only the active ancestry.", default: false },
  {
    key: "intensity",
    type: "enum",
    label: "Intensity",
    description: "Choose a restrained line strength.",
    default: "subtle",
    choices: [{ value: "subtle", label: "Subtle" }, { value: "standard", label: "Standard" }],
  },
  { key: "width", type: "number", label: "Width", description: "Line width.", default: 1, min: 1, max: 3, step: 0.5 },
  { key: "label", type: "string", label: "Label", description: "Optional label.", default: "", maxLength: 20 },
])!;

describe("plugin setting schemas", () => {
  it("parses bounded host-rendered definitions and materializes defaults", () => {
    expect(defaultPluginSettings(definitions)).toEqual({ "active-only": false, intensity: "subtle", width: 1, label: "" });
  });

  it("rejects unknown fields, duplicate keys, invalid defaults, and unsafe descriptions", () => {
    expect(() => parsePluginSettingDefinitions([
      { key: "x", type: "boolean", label: "X", description: "X", default: false, html: "<b>X</b>" },
    ])).toThrow(/unknown field html/);
    expect(() => parsePluginSettingDefinitions([
      { key: "same", type: "boolean", label: "One", description: "One", default: false },
      { key: "same", type: "boolean", label: "Two", description: "Two", default: true },
    ])).toThrow(/duplicate key/);
    expect(() => parsePluginSettingDefinitions([
      { key: "choice", type: "enum", label: "Choice", description: "Choice", default: "missing", choices: [
        { value: "a", label: "A" }, { value: "b", label: "B" },
      ] },
    ])).toThrow(/default must match/);
    expect(() => parsePluginSettingDefinitions([
      { key: "x", type: "boolean", label: "X", description: "bad\ntext", default: false },
    ])).toThrow(/plain text/);
  });

  it("keeps only schema-known values of the correct bounded type", () => {
    expect(validatePluginSettings(definitions, {
      "active-only": true,
      intensity: "loud",
      width: 99,
      label: "ok",
      smuggled: "ignored",
    })).toEqual({ "active-only": true, intensity: "subtle", width: 1, label: "ok" });
    expect(settingAccepts(definitions[2], 2.5)).toBe(true);
    expect(settingAccepts(definitions[2], Number.NaN)).toBe(false);
  });

  it("fails closed to defaults for malformed persisted JSON", () => {
    expect(parsePluginSettingsBlob(definitions, "not json")).toEqual(defaultPluginSettings(definitions));
  });
});
