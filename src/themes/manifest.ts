export const THEME_API_VERSION = "0.1" as const;

export const THEME_TOKENS = [
  "--ls-active-primary-color",
  "--ls-primary-background-color",
  "--ls-secondary-background-color",
  "--ls-tertiary-background-color",
  "--ls-quaternary-background-color",
  "--ls-primary-text-color",
  "--ls-secondary-text-color",
  "--ls-title-text-color",
  "--ls-link-text-color",
  "--ls-link-text-hover-color",
  "--ls-tag-text-color",
  "--ls-border-color",
  "--ls-guideline-color",
  "--ls-block-highlight-color",
  "--ls-block-bullet-color",
  "--ls-selection-background-color",
  "--ls-a-chosen-bg",
  "--ls-page-inline-code-bg-color",
  "--ls-page-inline-code-color",
  "--ls-page-mark-bg-color",
  "--ls-page-mark-color",
] as const;

export type ThemeToken = (typeof THEME_TOKENS)[number];
export type ThemeMode = "light" | "dark";
export type ThemeTokens = Partial<Record<ThemeToken, string>>;

export interface ThemePortProvenance {
  ecosystem: "logseq" | "obsidian" | "other";
  name: string;
  source: string;
  revision: string;
  license: string;
  authors: string[];
  relationship: "behavioral-port" | "source-derived";
}

export interface ThemeManifest {
  schemaVersion: 1;
  id: string;
  name: string;
  version: string;
  apiVersion: typeof THEME_API_VERSION;
  description: string;
  author: string;
  license: string;
  source: string;
  modes: Partial<Record<ThemeMode, ThemeTokens>>;
  screenshots: string[];
  portedFrom?: ThemePortProvenance;
  aiDevelopment?: "none" | "assisted" | "primary";
}

export class ThemeManifestError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ThemeManifestError";
  }
}

const ID_RE = /^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$/;
const VERSION_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;
const COLOR_RE = /^(?:#[0-9A-Fa-f]{3,8}|transparent|(?:rgb|rgba|hsl|hsla)\([0-9.,%+\- /]+\))$/;

function record(value: unknown, where: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new ThemeManifestError(`${where} must be an object`);
  return value as Record<string, unknown>;
}

function knownKeys(obj: Record<string, unknown>, where: string, allowed: readonly string[]) {
  const known = new Set(allowed);
  const unknown = Object.keys(obj).find((key) => !known.has(key));
  if (unknown) throw new ThemeManifestError(`${where} contains unknown field ${unknown}`);
}

function text(value: unknown, where: string, max: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > max || /[\u0000-\u001f]/.test(value)) {
    throw new ThemeManifestError(`${where} must be bounded plain text`);
  }
  return value;
}

function httpsUrl(value: unknown, where: string): string {
  const url = text(value, where, 500);
  try {
    if (new URL(url).protocol !== "https:") throw new Error();
  } catch {
    throw new ThemeManifestError(`${where} must be a public https URL`);
  }
  return url;
}

function parseTokens(value: unknown, where: string): ThemeTokens {
  const obj = record(value, where);
  knownKeys(obj, where, THEME_TOKENS);
  const tokens: ThemeTokens = {};
  for (const [key, candidate] of Object.entries(obj)) {
    if (typeof candidate !== "string" || candidate.length > 100 || !COLOR_RE.test(candidate)) {
      throw new ThemeManifestError(`${where}.${key} must be a literal color`);
    }
    tokens[key as ThemeToken] = candidate;
  }
  if (Object.keys(tokens).length === 0) throw new ThemeManifestError(`${where} must not be empty`);
  return tokens;
}

function parseProvenance(value: unknown): ThemePortProvenance | undefined {
  if (value === undefined) return undefined;
  const obj = record(value, "portedFrom");
  knownKeys(obj, "portedFrom", ["ecosystem", "name", "source", "revision", "license", "authors", "relationship"]);
  if (obj.ecosystem !== "logseq" && obj.ecosystem !== "obsidian" && obj.ecosystem !== "other") {
    throw new ThemeManifestError("portedFrom.ecosystem is unsupported");
  }
  if (obj.relationship !== "behavioral-port" && obj.relationship !== "source-derived") {
    throw new ThemeManifestError("portedFrom.relationship is unsupported");
  }
  if (!Array.isArray(obj.authors) || obj.authors.length === 0 || obj.authors.length > 32) {
    throw new ThemeManifestError("portedFrom.authors must contain 1 to 32 entries");
  }
  return {
    ecosystem: obj.ecosystem,
    name: text(obj.name, "portedFrom.name", 120),
    source: httpsUrl(obj.source, "portedFrom.source"),
    revision: text(obj.revision, "portedFrom.revision", 160),
    license: text(obj.license, "portedFrom.license", 80),
    authors: obj.authors.map((author, index) => text(author, `portedFrom.authors[${index}]`, 160)),
    relationship: obj.relationship,
  };
}

export function parseThemeManifest(value: unknown): ThemeManifest {
  const obj = record(value, "theme manifest");
  knownKeys(obj, "theme manifest", [
    "schemaVersion", "id", "name", "version", "apiVersion", "description", "author", "license",
    "source", "modes", "screenshots", "portedFrom", "aiDevelopment",
  ]);
  if (obj.schemaVersion !== 1) throw new ThemeManifestError("schemaVersion must be 1");
  if (obj.apiVersion !== THEME_API_VERSION) throw new ThemeManifestError(`apiVersion must be ${THEME_API_VERSION}`);
  const id = text(obj.id, "id", 64);
  if (!ID_RE.test(id) || !id.includes(".")) throw new ThemeManifestError("id must be a lowercase dotted identifier");
  const version = text(obj.version, "version", 64);
  if (!VERSION_RE.test(version)) throw new ThemeManifestError("version must be SemVer");
  const modesObj = record(obj.modes, "modes");
  knownKeys(modesObj, "modes", ["light", "dark"]);
  if (modesObj.light === undefined && modesObj.dark === undefined) throw new ThemeManifestError("modes must include light or dark");
  if (!Array.isArray(obj.screenshots) || obj.screenshots.length > 6) {
    throw new ThemeManifestError("screenshots must be an array of at most 6 URLs");
  }
  const aiDevelopment = obj.aiDevelopment;
  if (aiDevelopment !== undefined && aiDevelopment !== "none" && aiDevelopment !== "assisted" && aiDevelopment !== "primary") {
    throw new ThemeManifestError("aiDevelopment is unsupported");
  }
  return {
    schemaVersion: 1,
    id,
    name: text(obj.name, "name", 80),
    version,
    apiVersion: THEME_API_VERSION,
    description: text(obj.description, "description", 500),
    author: text(obj.author, "author", 160),
    license: text(obj.license, "license", 80),
    source: httpsUrl(obj.source, "source"),
    modes: {
      ...(modesObj.light === undefined ? {} : { light: parseTokens(modesObj.light, "modes.light") }),
      ...(modesObj.dark === undefined ? {} : { dark: parseTokens(modesObj.dark, "modes.dark") }),
    },
    screenshots: obj.screenshots.map((url, index) => httpsUrl(url, `screenshots[${index}]`)),
    ...(obj.portedFrom === undefined ? {} : { portedFrom: parseProvenance(obj.portedFrom) }),
    ...(aiDevelopment === undefined ? {} : { aiDevelopment }),
  };
}

export function themeVersionKey(manifest: Pick<ThemeManifest, "id" | "version">): string {
  return `${manifest.id}@${manifest.version}`;
}

export function themeManifestCss(manifest: ThemeManifest): string {
  return (["light", "dark"] as const).flatMap((mode) => {
    const tokens = manifest.modes[mode];
    if (!tokens) return [];
    const declarations = Object.entries(tokens).map(([token, value]) => `  ${token}: ${value};`).join("\n");
    return [`html[data-theme="${mode}"] {\n${declarations}\n}`];
  }).join("\n\n");
}
