export const PLUGIN_API_VERSION = "0.1" as const;

export const PLUGIN_PLATFORMS = ["desktop", "android", "ios"] as const;
export type PluginPlatform = (typeof PLUGIN_PLATFORMS)[number];

export const PLUGIN_CAPABILITIES = [
  "commands.register",
  "slash-commands.register",
  "block-decorations.register",
  "graph.read.visible",
  "graph.write.block",
  "settings.read",
  "settings.write",
] as const;
export type PluginCapability = (typeof PLUGIN_CAPABILITIES)[number];

export interface PluginCommandContribution {
  id: string;
  title: string;
  description?: string;
  platforms?: PluginPlatform[];
}

export interface PluginSlashCommandContribution {
  id: string;
  title: string;
  insertText?: string;
  platforms?: PluginPlatform[];
}

export interface PluginBlockDecorationContribution {
  id: string;
  kind: "thread-lines" | "badge";
  platforms?: PluginPlatform[];
}

export interface PluginManifest {
  schemaVersion: 1;
  id: string;
  name: string;
  version: string;
  apiVersion: typeof PLUGIN_API_VERSION;
  description: string;
  author: string;
  license: string;
  source: string;
  entry: string;
  platforms: PluginPlatform[];
  capabilities: PluginCapability[];
  contributions?: {
    commands?: PluginCommandContribution[];
    slashCommands?: PluginSlashCommandContribution[];
    blockDecorations?: PluginBlockDecorationContribution[];
  };
  aiDevelopment?: "none" | "assisted" | "primary";
}

export class PluginManifestError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginManifestError";
  }
}

const ID_RE = /^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$/;
const VERSION_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;
const CONTRIBUTION_ID_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const SAFE_ENTRY_RE = /^[A-Za-z0-9][A-Za-z0-9._/-]*\.wasm$/;

function record(value: unknown, where: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new PluginManifestError(`${where} must be an object`);
  }
  return value as Record<string, unknown>;
}

function knownKeys(obj: Record<string, unknown>, where: string, allowed: readonly string[]) {
  const known = new Set(allowed);
  const unknown = Object.keys(obj).find((key) => !known.has(key));
  if (unknown) throw new PluginManifestError(`${where} contains unknown field ${unknown}`);
}

function stringField(value: unknown, where: string, max: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > max) {
    throw new PluginManifestError(`${where} must be a non-empty string of at most ${max} characters`);
  }
  return value;
}

function stringArray<T extends string>(
  value: unknown,
  where: string,
  allowed: readonly T[],
  fallback?: T[]
): T[] {
  if (value === undefined && fallback) return fallback;
  if (!Array.isArray(value)) throw new PluginManifestError(`${where} must be an array`);
  const result = value.map((item) => {
    if (typeof item !== "string" || !allowed.includes(item as T)) {
      throw new PluginManifestError(`${where} contains unsupported value ${JSON.stringify(item)}`);
    }
    return item as T;
  });
  if (new Set(result).size !== result.length) throw new PluginManifestError(`${where} contains duplicates`);
  return result;
}

function platforms(value: unknown, where: string, fallback?: PluginPlatform[]): PluginPlatform[] {
  const parsed = stringArray(value, where, PLUGIN_PLATFORMS, fallback);
  if (parsed.length === 0) throw new PluginManifestError(`${where} must not be empty`);
  return parsed;
}

function optionalPlatforms(value: unknown, where: string): PluginPlatform[] | undefined {
  return value === undefined ? undefined : platforms(value, where);
}

function contributionId(value: unknown, where: string): string {
  const id = stringField(value, where, 64);
  if (!CONTRIBUTION_ID_RE.test(id)) throw new PluginManifestError(`${where} has an invalid id`);
  return id;
}

function parseCommands(value: unknown): PluginCommandContribution[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new PluginManifestError("contributions.commands must be an array");
  return value.map((item, index) => {
    const obj = record(item, `contributions.commands[${index}]`);
    knownKeys(obj, `contributions.commands[${index}]`, ["id", "title", "description", "platforms"]);
    return {
      id: contributionId(obj.id, `contributions.commands[${index}].id`),
      title: stringField(obj.title, `contributions.commands[${index}].title`, 80),
      ...(obj.description === undefined
        ? {}
        : { description: stringField(obj.description, `contributions.commands[${index}].description`, 240) }),
      ...(obj.platforms === undefined
        ? {}
        : { platforms: optionalPlatforms(obj.platforms, `contributions.commands[${index}].platforms`) }),
    };
  });
}

function parseSlashCommands(value: unknown): PluginSlashCommandContribution[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new PluginManifestError("contributions.slashCommands must be an array");
  return value.map((item, index) => {
    const obj = record(item, `contributions.slashCommands[${index}]`);
    knownKeys(obj, `contributions.slashCommands[${index}]`, ["id", "title", "insertText", "platforms"]);
    return {
      id: contributionId(obj.id, `contributions.slashCommands[${index}].id`),
      title: stringField(obj.title, `contributions.slashCommands[${index}].title`, 80),
      ...(obj.insertText === undefined
        ? {}
        : { insertText: stringField(obj.insertText, `contributions.slashCommands[${index}].insertText`, 4_096) }),
      ...(obj.platforms === undefined
        ? {}
        : { platforms: optionalPlatforms(obj.platforms, `contributions.slashCommands[${index}].platforms`) }),
    };
  });
}

function parseDecorations(value: unknown): PluginBlockDecorationContribution[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new PluginManifestError("contributions.blockDecorations must be an array");
  return value.map((item, index) => {
    const obj = record(item, `contributions.blockDecorations[${index}]`);
    knownKeys(obj, `contributions.blockDecorations[${index}]`, ["id", "kind", "platforms"]);
    if (obj.kind !== "thread-lines" && obj.kind !== "badge") {
      throw new PluginManifestError(`contributions.blockDecorations[${index}].kind is unsupported`);
    }
    return {
      id: contributionId(obj.id, `contributions.blockDecorations[${index}].id`),
      kind: obj.kind,
      ...(obj.platforms === undefined
        ? {}
        : { platforms: optionalPlatforms(obj.platforms, `contributions.blockDecorations[${index}].platforms`) }),
    };
  });
}

function assertUniqueContributions(manifest: PluginManifest) {
  const seen = new Set<string>();
  for (const [kind, entries] of Object.entries(manifest.contributions ?? {})) {
    for (const entry of entries ?? []) {
      const key = `${kind}:${entry.id}`;
      if (seen.has(key)) throw new PluginManifestError(`duplicate contribution ${key}`);
      seen.add(key);
    }
  }
}

function assertContributionCapabilities(manifest: PluginManifest) {
  const required: Array<[boolean, PluginCapability]> = [
    [!!manifest.contributions?.commands?.length, "commands.register"],
    [!!manifest.contributions?.slashCommands?.length, "slash-commands.register"],
    [!!manifest.contributions?.blockDecorations?.length, "block-decorations.register"],
  ];
  for (const [used, capability] of required) {
    if (used && !manifest.capabilities.includes(capability)) {
      throw new PluginManifestError(`contribution requires capability ${capability}`);
    }
  }
}

/** Strictly parse the untrusted manifest before reading or compiling its entry. */
export function parsePluginManifest(value: unknown): PluginManifest {
  const obj = record(value, "manifest");
  knownKeys(obj, "manifest", [
    "schemaVersion", "id", "name", "version", "apiVersion", "description", "author", "license",
    "source", "entry", "platforms", "capabilities", "contributions", "aiDevelopment",
  ]);
  if (obj.schemaVersion !== 1) throw new PluginManifestError("schemaVersion must be 1");
  if (obj.apiVersion !== PLUGIN_API_VERSION) {
    throw new PluginManifestError(`apiVersion must be ${PLUGIN_API_VERSION}`);
  }
  const id = stringField(obj.id, "id", 64);
  if (!ID_RE.test(id) || !id.includes(".")) throw new PluginManifestError("id must be a lowercase dotted identifier");
  const version = stringField(obj.version, "version", 64);
  if (!VERSION_RE.test(version)) throw new PluginManifestError("version must be SemVer");
  const entry = stringField(obj.entry, "entry", 160);
  if (!SAFE_ENTRY_RE.test(entry) || entry.startsWith("/") || entry.split("/").includes("..")) {
    throw new PluginManifestError("entry must be a relative .wasm path without traversal");
  }
  const contributionsObj = obj.contributions === undefined ? undefined : record(obj.contributions, "contributions");
  if (contributionsObj) knownKeys(contributionsObj, "contributions", ["commands", "slashCommands", "blockDecorations"]);
  const aiDevelopment = obj.aiDevelopment;
  if (aiDevelopment !== undefined && aiDevelopment !== "none" && aiDevelopment !== "assisted" && aiDevelopment !== "primary") {
    throw new PluginManifestError("aiDevelopment is unsupported");
  }
  const manifest: PluginManifest = {
    schemaVersion: 1,
    id,
    name: stringField(obj.name, "name", 80),
    version,
    apiVersion: PLUGIN_API_VERSION,
    description: stringField(obj.description, "description", 500),
    author: stringField(obj.author, "author", 160),
    license: stringField(obj.license, "license", 80),
    source: stringField(obj.source, "source", 500),
    entry,
    platforms: platforms(obj.platforms, "platforms", ["desktop"]),
    capabilities: stringArray(obj.capabilities, "capabilities", PLUGIN_CAPABILITIES),
    ...(contributionsObj
      ? {
          contributions: {
            commands: parseCommands(contributionsObj.commands),
            slashCommands: parseSlashCommands(contributionsObj.slashCommands),
            blockDecorations: parseDecorations(contributionsObj.blockDecorations),
          },
        }
      : {}),
    ...(aiDevelopment === undefined ? {} : { aiDevelopment }),
  };
  assertUniqueContributions(manifest);
  assertContributionCapabilities(manifest);
  return manifest;
}

export function supportsPlatform(
  manifest: PluginManifest,
  platform: PluginPlatform,
  contributionPlatforms?: PluginPlatform[]
): boolean {
  return manifest.platforms.includes(platform) && (!contributionPlatforms || contributionPlatforms.includes(platform));
}
