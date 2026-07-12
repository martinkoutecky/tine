export type PluginSettingValue = string | number | boolean;
export type PluginSettings = Record<string, PluginSettingValue>;

interface PluginSettingBase {
  key: string;
  label: string;
  description: string;
}

export interface PluginBooleanSetting extends PluginSettingBase {
  type: "boolean";
  default: boolean;
}

export interface PluginEnumSetting extends PluginSettingBase {
  type: "enum";
  default: string;
  choices: Array<{ value: string; label: string }>;
}

export interface PluginNumberSetting extends PluginSettingBase {
  type: "number";
  default: number;
  min: number;
  max: number;
  step?: number;
}

export interface PluginStringSetting extends PluginSettingBase {
  type: "string";
  default: string;
  maxLength: number;
}

export type PluginSettingDefinition =
  | PluginBooleanSetting
  | PluginEnumSetting
  | PluginNumberSetting
  | PluginStringSetting;

export class PluginSettingsError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginSettingsError";
  }
}

const KEY_RE = /^[a-z][a-z0-9._-]{0,79}$/;
const CHOICE_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/;

function record(value: unknown, where: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new PluginSettingsError(`${where} must be an object`);
  }
  return value as Record<string, unknown>;
}

function knownKeys(obj: Record<string, unknown>, where: string, allowed: readonly string[]) {
  const known = new Set(allowed);
  const unknown = Object.keys(obj).find((key) => !known.has(key));
  if (unknown) throw new PluginSettingsError(`${where} contains unknown field ${unknown}`);
}

function text(value: unknown, where: string, max: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > max || /[\u0000-\u001f]/.test(value)) {
    throw new PluginSettingsError(`${where} must be plain text of at most ${max} characters`);
  }
  return value;
}

function finite(value: unknown, where: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new PluginSettingsError(`${where} must be a finite number`);
  }
  return value;
}

export function parsePluginSettingDefinitions(value: unknown): PluginSettingDefinition[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new PluginSettingsError("settings must be an array");
  if (value.length > 64) throw new PluginSettingsError("settings must contain at most 64 entries");
  const seen = new Set<string>();
  return value.map((candidate, index): PluginSettingDefinition => {
    const where = `settings[${index}]`;
    const obj = record(candidate, where);
    const key = text(obj.key, `${where}.key`, 80);
    if (!KEY_RE.test(key)) throw new PluginSettingsError(`${where}.key is invalid`);
    if (seen.has(key)) throw new PluginSettingsError(`settings contains duplicate key ${key}`);
    seen.add(key);
    const base = {
      key,
      label: text(obj.label, `${where}.label`, 80),
      description: text(obj.description, `${where}.description`, 300),
    };
    switch (obj.type) {
      case "boolean":
        knownKeys(obj, where, ["key", "type", "label", "description", "default"]);
        if (typeof obj.default !== "boolean") throw new PluginSettingsError(`${where}.default must be boolean`);
        return { ...base, type: "boolean", default: obj.default };
      case "enum": {
        knownKeys(obj, where, ["key", "type", "label", "description", "default", "choices"]);
        if (!Array.isArray(obj.choices) || obj.choices.length < 2 || obj.choices.length > 32) {
          throw new PluginSettingsError(`${where}.choices must contain 2 to 32 entries`);
        }
        const choices = obj.choices.map((choice, choiceIndex) => {
          const parsed = record(choice, `${where}.choices[${choiceIndex}]`);
          knownKeys(parsed, `${where}.choices[${choiceIndex}]`, ["value", "label"]);
          const choiceValue = text(parsed.value, `${where}.choices[${choiceIndex}].value`, 80);
          if (!CHOICE_RE.test(choiceValue)) throw new PluginSettingsError(`${where}.choices[${choiceIndex}].value is invalid`);
          return { value: choiceValue, label: text(parsed.label, `${where}.choices[${choiceIndex}].label`, 80) };
        });
        if (new Set(choices.map((choice) => choice.value)).size !== choices.length) {
          throw new PluginSettingsError(`${where}.choices contains duplicate values`);
        }
        const defaultValue = text(obj.default, `${where}.default`, 80);
        if (!choices.some((choice) => choice.value === defaultValue)) {
          throw new PluginSettingsError(`${where}.default must match a choice`);
        }
        return { ...base, type: "enum", default: defaultValue, choices };
      }
      case "number": {
        knownKeys(obj, where, ["key", "type", "label", "description", "default", "min", "max", "step"]);
        const min = finite(obj.min, `${where}.min`);
        const max = finite(obj.max, `${where}.max`);
        const defaultValue = finite(obj.default, `${where}.default`);
        if (min > max || defaultValue < min || defaultValue > max) {
          throw new PluginSettingsError(`${where} has inconsistent numeric bounds`);
        }
        const step = obj.step === undefined ? undefined : finite(obj.step, `${where}.step`);
        if (step !== undefined && step <= 0) throw new PluginSettingsError(`${where}.step must be positive`);
        return { ...base, type: "number", default: defaultValue, min, max, ...(step === undefined ? {} : { step }) };
      }
      case "string": {
        knownKeys(obj, where, ["key", "type", "label", "description", "default", "maxLength"]);
        const maxLength = finite(obj.maxLength, `${where}.maxLength`);
        if (!Number.isInteger(maxLength) || maxLength < 1 || maxLength > 4096) {
          throw new PluginSettingsError(`${where}.maxLength must be an integer from 1 to 4096`);
        }
        if (typeof obj.default !== "string" || obj.default.length > maxLength) {
          throw new PluginSettingsError(`${where}.default exceeds maxLength`);
        }
        return { ...base, type: "string", default: obj.default, maxLength };
      }
      default:
        throw new PluginSettingsError(`${where}.type is unsupported`);
    }
  });
}

export function settingAccepts(definition: PluginSettingDefinition, value: unknown): value is PluginSettingValue {
  switch (definition.type) {
    case "boolean":
      return typeof value === "boolean";
    case "enum":
      return typeof value === "string" && definition.choices.some((choice) => choice.value === value);
    case "number":
      return typeof value === "number" && Number.isFinite(value) && value >= definition.min && value <= definition.max;
    case "string":
      return typeof value === "string" && value.length <= definition.maxLength;
  }
}

export function defaultPluginSettings(definitions: readonly PluginSettingDefinition[] | undefined): PluginSettings {
  return Object.fromEntries((definitions ?? []).map((definition) => [definition.key, definition.default]));
}

export function validatePluginSettings(
  definitions: readonly PluginSettingDefinition[] | undefined,
  value: unknown
): PluginSettings {
  const raw = value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
  const validated = defaultPluginSettings(definitions);
  for (const definition of definitions ?? []) {
    const candidate = raw[definition.key];
    if (settingAccepts(definition, candidate)) validated[definition.key] = candidate;
  }
  return validated;
}

export function parsePluginSettingsBlob(
  definitions: readonly PluginSettingDefinition[] | undefined,
  textValue: string
): PluginSettings {
  try {
    return validatePluginSettings(definitions, JSON.parse(textValue));
  } catch {
    return defaultPluginSettings(definitions);
  }
}
