// Compare two exact-byte graph manifests (`tine-graph-bytes`, written by the
// backend's graph verifier). Pure; parsing is strict because the other device's
// report is pasted text: unsafe paths, bad digests and duplicates are refused.
/** What `createGraphVerification` returns: the report text plus its headline. */
export interface GraphVerificationReport {
  text: string;
  suggestedFileName: string;
  totalFiles: number;
  totalBytes: number;
  aggregateDigest?: string;
  complete: boolean;
}

export interface GraphVerificationProgress {
  operationId: string;
  processed: number;
  total: number;
}

export interface GraphVerificationFile {
  path: string;
  length: number;
  digest: string;
}

export interface GraphVerificationManifest {
  schemaVersion: 1;
  tool: "tine-graph-bytes";
  algorithm: "sha256";
  complete: boolean;
  generatedAtUnixMs: number;
  files: GraphVerificationFile[];
  aggregateDigest?: string;
  errors: Array<{ path?: string; detail: string }>;
}

export interface GraphVerificationComparison {
  matches: boolean;
  incomplete: boolean;
  localOnly: string[];
  otherOnly: string[];
  changed: string[];
}

const SHA256 = /^[0-9a-f]{64}$/;

function isSafeRelativePath(path: string): boolean {
  if (!path || path.startsWith("/") || path.includes("\\") || path.includes("\0")) return false;
  return path.split("/").every((part) => part !== "" && part !== "." && part !== "..");
}

export function parseGraphVerificationManifest(text: string): GraphVerificationManifest {
  const value: unknown = JSON.parse(text);
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("The comparison report is not a JSON object.");
  }
  const manifest = value as Record<string, unknown>;
  if (manifest.schemaVersion !== 1 || manifest.tool !== "tine-graph-bytes" || manifest.algorithm !== "sha256") {
    throw new Error("The comparison report has an unsupported format.");
  }
  if (typeof manifest.complete !== "boolean" || !Number.isSafeInteger(manifest.generatedAtUnixMs)) {
    throw new Error("The comparison report is missing required metadata.");
  }
  if (!Array.isArray(manifest.files) || !Array.isArray(manifest.errors)) {
    throw new Error("The comparison report has invalid file or error entries.");
  }

  const files = manifest.files.map((entry): GraphVerificationFile => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error("The comparison report has an invalid file entry.");
    }
    const file = entry as Record<string, unknown>;
    if (typeof file.path !== "string" || !isSafeRelativePath(file.path)) {
      throw new Error("The comparison report contains an unsafe file path.");
    }
    if (!Number.isSafeInteger(file.length) || (file.length as number) < 0) {
      throw new Error(`The comparison report has an invalid length for ${file.path}.`);
    }
    if (typeof file.digest !== "string" || !SHA256.test(file.digest)) {
      throw new Error(`The comparison report has an invalid digest for ${file.path}.`);
    }
    return { path: file.path, length: file.length as number, digest: file.digest };
  });
  files.sort((left, right) => left.path.localeCompare(right.path));
  if (files.some((file, index) => index > 0 && file.path === files[index - 1].path)) {
    throw new Error("The comparison report contains duplicate file paths.");
  }
  if (manifest.aggregateDigest !== undefined &&
      (typeof manifest.aggregateDigest !== "string" || !SHA256.test(manifest.aggregateDigest))) {
    throw new Error("The comparison report has an invalid aggregate digest.");
  }
  for (const entry of manifest.errors) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry) ||
        typeof (entry as Record<string, unknown>).detail !== "string") {
      throw new Error("The comparison report has an invalid error entry.");
    }
  }
  return {
    schemaVersion: 1,
    tool: "tine-graph-bytes",
    algorithm: "sha256",
    complete: manifest.complete,
    generatedAtUnixMs: manifest.generatedAtUnixMs as number,
    files,
    aggregateDigest: manifest.aggregateDigest as string | undefined,
    errors: manifest.errors as Array<{ path?: string; detail: string }>,
  };
}

export function compareGraphVerificationManifests(
  local: GraphVerificationManifest,
  other: GraphVerificationManifest,
): GraphVerificationComparison {
  if (!local.complete || !other.complete) {
    return { matches: false, incomplete: true, localOnly: [], otherOnly: [], changed: [] };
  }
  const localFiles = new Map(local.files.map((file) => [file.path, file]));
  const otherFiles = new Map(other.files.map((file) => [file.path, file]));
  const localOnly = [...localFiles.keys()].filter((path) => !otherFiles.has(path)).sort();
  const otherOnly = [...otherFiles.keys()].filter((path) => !localFiles.has(path)).sort();
  const changed = [...localFiles.keys()]
    .filter((path) => {
      const remote = otherFiles.get(path);
      const current = localFiles.get(path)!;
      return remote !== undefined && (remote.length !== current.length || remote.digest !== current.digest);
    })
    .sort();
  return {
    matches: localOnly.length === 0 && otherOnly.length === 0 && changed.length === 0,
    incomplete: false,
    localOnly,
    otherOnly,
    changed,
  };
}
