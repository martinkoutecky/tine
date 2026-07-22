import type { PluginCapability, PluginPlatform } from "./manifest";

export const PLUGIN_PROTOCOL_VERSION = 2 as const;

export interface PluginBlockSnapshot {
  id: string;
  raw: string;
  parentId: string | null;
  depth: number;
  format?: "md" | "org";
}

export type PluginEvent =
  | {
      protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
      kind: "activate";
      platform: PluginPlatform;
      capabilities: PluginCapability[];
      settings: Record<string, string | number | boolean | null>;
    }
  | {
      protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
      kind: "settings-changed";
      settings: Record<string, string | number | boolean | null>;
      changedKeys: string[];
    }
  | {
      protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
      kind: "command";
      contributionId: string;
      focusedBlock?: PluginBlockSnapshot;
    }
  | {
      protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
      kind: "slash-command";
      contributionId: string;
      focusedBlock: PluginBlockSnapshot;
    }
  | {
      protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
      kind: "decorate-blocks";
      contributionId: string;
      blocks: PluginBlockSnapshot[];
    };

export type PluginEffect =
  | { kind: "notice"; message: string; level?: "info" | "warning" | "error" }
  | { kind: "replace-block-text"; blockId: string; expectedRaw: string; raw: string }
  | { kind: "insert-at-caret"; text: string }
  | {
      kind: "block-decoration";
      blockId: string;
      decoration: "thread-line" | "badge";
      label?: string;
      tone?: "neutral" | "accent" | "warning";
    }
  | { kind: "set-setting"; key: string; value: string | number | boolean | null };

export interface PluginResponse {
  protocolVersion: typeof PLUGIN_PROTOCOL_VERSION;
  effects: PluginEffect[];
}

export const MAX_EVENT_BYTES = 256 * 1024;
export const MAX_RESPONSE_BYTES = 256 * 1024;
export const MAX_EFFECTS = 512;

export class PluginProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginProtocolError";
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function boundedString(value: unknown, field: string, max: number): string {
  if (typeof value !== "string" || value.length > max) throw new PluginProtocolError(`${field} is invalid`);
  return value;
}

export function parsePluginResponse(value: unknown): PluginResponse {
  if (!isRecord(value) || value.protocolVersion !== PLUGIN_PROTOCOL_VERSION || !Array.isArray(value.effects)) {
    throw new PluginProtocolError("guest returned an invalid response envelope");
  }
  if (value.effects.length > MAX_EFFECTS) throw new PluginProtocolError("guest returned too many effects");
  const effects = value.effects.map((candidate, index): PluginEffect => {
    if (!isRecord(candidate) || typeof candidate.kind !== "string") {
      throw new PluginProtocolError(`effect ${index} is invalid`);
    }
    switch (candidate.kind) {
      case "notice": {
        const level = candidate.level;
        if (level !== undefined && level !== "info" && level !== "warning" && level !== "error") {
          throw new PluginProtocolError(`effect ${index} has invalid notice level`);
        }
        return { kind: "notice", message: boundedString(candidate.message, `effect ${index}.message`, 500), ...(level ? { level } : {}) };
      }
      case "replace-block-text":
        return {
          kind: "replace-block-text",
          blockId: boundedString(candidate.blockId, `effect ${index}.blockId`, 160),
          expectedRaw: boundedString(candidate.expectedRaw, `effect ${index}.expectedRaw`, 128 * 1024),
          raw: boundedString(candidate.raw, `effect ${index}.raw`, 128 * 1024),
        };
      case "insert-at-caret":
        return { kind: "insert-at-caret", text: boundedString(candidate.text, `effect ${index}.text`, 32 * 1024) };
      case "block-decoration": {
        if (candidate.decoration !== "thread-line" && candidate.decoration !== "badge") {
          throw new PluginProtocolError(`effect ${index} has invalid decoration`);
        }
        const tone = candidate.tone;
        if (tone !== undefined && tone !== "neutral" && tone !== "accent" && tone !== "warning") {
          throw new PluginProtocolError(`effect ${index} has invalid tone`);
        }
        return {
          kind: "block-decoration",
          blockId: boundedString(candidate.blockId, `effect ${index}.blockId`, 160),
          decoration: candidate.decoration,
          ...(candidate.label === undefined ? {} : { label: boundedString(candidate.label, `effect ${index}.label`, 80) }),
          ...(tone === undefined ? {} : { tone }),
        };
      }
      case "set-setting": {
        const setting = candidate.value;
        if (setting !== null && typeof setting !== "string" && typeof setting !== "number" && typeof setting !== "boolean") {
          throw new PluginProtocolError(`effect ${index} has invalid setting value`);
        }
        return {
          kind: "set-setting",
          key: boundedString(candidate.key, `effect ${index}.key`, 80),
          value: setting,
        };
      }
      default:
        throw new PluginProtocolError(`effect ${index} has unsupported kind`);
    }
  });
  return { protocolVersion: PLUGIN_PROTOCOL_VERSION, effects };
}
