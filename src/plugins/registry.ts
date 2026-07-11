import { createSignal } from "solid-js";
import { backend } from "../backend";
import {
  PLUGIN_API_VERSION,
  PLUGIN_CAPABILITIES,
  PLUGIN_PLATFORMS,
  parsePluginManifest,
  type PluginCapability,
  type PluginPlatform,
} from "./manifest";
import { pluginManager } from "./manager";

export const COMMUNITY_REGISTRY_URL =
  "https://raw.githubusercontent.com/martinkoutecky/tine-plugin-registry/main/index.json";
const MAX_INDEX_BYTES = 2 * 1024 * 1024;
const MAX_WASM_BYTES = 8 * 1024 * 1024;

export interface RegistryVersion {
  version: string;
  apiVersion: string;
  platforms: Array<"desktop" | "android" | "ios">;
  capabilities: string[];
  sha256: string;
  manifestSha256: string;
  manifestUrl: string;
  wasmUrl: string;
  audit: { status: string; url: string };
  publishedAt: string;
}

export interface RegistryPlugin {
  id: string;
  name: string;
  description: string;
  source: string;
  license: string;
  aiDevelopment: "none" | "assisted" | "primary";
  versions: RegistryVersion[];
}

interface RegistryIndex {
  schemaVersion: 1;
  generatedAt: string;
  plugins: RegistryPlugin[];
  revocations: Array<{ id: string; version: string; severity: string; reason: string; revokedAt: string }>;
}

const [communityPlugins, setCommunityPlugins] = createSignal<RegistryPlugin[]>([]);
const [registryState, setRegistryState] = createSignal<"idle" | "loading" | "ready" | "offline" | "invalid">("idle");
export { communityPlugins, registryState };

function object(value: unknown, where: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${where} is invalid`);
  return value as Record<string, unknown>;
}

function knownKeys(value: Record<string, unknown>, where: string, allowed: readonly string[]): void {
  const known = new Set(allowed);
  const unknown = Object.keys(value).find((key) => !known.has(key));
  if (unknown) throw new Error(`${where} contains unknown field ${unknown}`);
}

function text(value: unknown, where: string, max = 500): string {
  if (typeof value !== "string" || value.length === 0 || value.length > max) throw new Error(`${where} is invalid`);
  return value;
}

function https(value: unknown, where: string): string {
  const result = text(value, where, 1_000);
  const url = new URL(result);
  if (url.protocol !== "https:") throw new Error(`${where} must use https`);
  return result;
}

export function parseRegistryIndex(value: unknown): RegistryIndex {
  const root = object(value, "registry");
  knownKeys(root, "registry", ["schemaVersion", "generatedAt", "plugins", "revocations"]);
  if (root.schemaVersion !== 1 || !Array.isArray(root.plugins) || !Array.isArray(root.revocations)) {
    throw new Error("registry envelope is incompatible");
  }
  const ids = new Set<string>();
  const plugins = root.plugins.map((candidate, pluginIndex): RegistryPlugin => {
    const item = object(candidate, `plugins[${pluginIndex}]`);
    knownKeys(item, `plugins[${pluginIndex}]`, ["id", "name", "description", "source", "license", "aiDevelopment", "versions"]);
    const id = text(item.id, `plugins[${pluginIndex}].id`, 64);
    if (!/^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$/.test(id) || !id.includes(".")) {
      throw new Error(`plugins[${pluginIndex}].id is invalid`);
    }
    if (ids.has(id)) throw new Error(`duplicate registry plugin ${id}`);
    ids.add(id);
    if (!Array.isArray(item.versions) || item.versions.length === 0) throw new Error(`${id} has no versions`);
    const seenVersions = new Set<string>();
    const versions = item.versions.map((candidateVersion, versionIndex): RegistryVersion => {
      const version = object(candidateVersion, `${id}.versions[${versionIndex}]`);
      knownKeys(version, `${id}.versions[${versionIndex}]`, [
        "version", "apiVersion", "platforms", "capabilities", "sha256", "manifestSha256",
        "manifestUrl", "wasmUrl", "audit", "publishedAt",
      ]);
      const sha256 = text(version.sha256, `${id}.sha256`, 64);
      if (!/^[0-9a-f]{64}$/.test(sha256)) throw new Error(`${id} has an invalid digest`);
      const manifestSha256 = text(version.manifestSha256, `${id}.manifestSha256`, 64);
      if (!/^[0-9a-f]{64}$/.test(manifestSha256)) throw new Error(`${id} has an invalid manifest digest`);
      if (!Array.isArray(version.platforms) || !Array.isArray(version.capabilities)) {
        throw new Error(`${id} version metadata is invalid`);
      }
      const parsedVersion = text(version.version, `${id}.version`, 64);
      if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/.test(parsedVersion)) {
        throw new Error(`${id} has an invalid version`);
      }
      if (seenVersions.has(parsedVersion)) throw new Error(`${id} has duplicate version ${parsedVersion}`);
      seenVersions.add(parsedVersion);
      if (version.apiVersion !== PLUGIN_API_VERSION) throw new Error(`${id} has an incompatible API version`);
      const parsedPlatforms = version.platforms.map((platform) => {
        if (typeof platform !== "string" || !PLUGIN_PLATFORMS.includes(platform as PluginPlatform)) {
          throw new Error(`${id} has an invalid platform`);
        }
        return platform as PluginPlatform;
      });
      if (parsedPlatforms.length === 0 || new Set(parsedPlatforms).size !== parsedPlatforms.length) {
        throw new Error(`${id} has invalid platforms`);
      }
      const parsedCapabilities = version.capabilities.map((capability) => {
        if (typeof capability !== "string" || !PLUGIN_CAPABILITIES.includes(capability as PluginCapability)) {
          throw new Error(`${id} has an invalid capability`);
        }
        return capability;
      });
      if (new Set(parsedCapabilities).size !== parsedCapabilities.length) throw new Error(`${id} has duplicate capabilities`);
      const audit = object(version.audit, `${id}.audit`);
      knownKeys(audit, `${id}.audit`, ["status", "url"]);
      if (audit.status !== "passed") throw new Error(`${id} does not have a passing audit`);
      return {
        version: parsedVersion,
        apiVersion: PLUGIN_API_VERSION,
        platforms: parsedPlatforms,
        capabilities: parsedCapabilities,
        sha256,
        manifestSha256,
        manifestUrl: https(version.manifestUrl, `${id}.manifestUrl`),
        wasmUrl: https(version.wasmUrl, `${id}.wasmUrl`),
        audit: { status: text(audit.status, `${id}.audit.status`, 40), url: https(audit.url, `${id}.audit.url`) },
        publishedAt: text(version.publishedAt, `${id}.publishedAt`, 80),
      };
    });
    const ai = item.aiDevelopment;
    if (ai !== "none" && ai !== "assisted" && ai !== "primary") throw new Error(`${id} has invalid AI provenance`);
    return {
      id,
      name: text(item.name, `${id}.name`, 80),
      description: text(item.description, `${id}.description`, 500),
      source: https(item.source, `${id}.source`),
      license: text(item.license, `${id}.license`, 80),
      aiDevelopment: ai,
      versions,
    };
  });
  const revocations = root.revocations.map((candidate, index) => {
    const item = object(candidate, `revocations[${index}]`);
    knownKeys(item, `revocations[${index}]`, ["id", "version", "severity", "reason", "revokedAt"]);
    return {
      id: text(item.id, `revocations[${index}].id`, 64),
      version: text(item.version, `revocations[${index}].version`, 64),
      severity: text(item.severity, `revocations[${index}].severity`, 40),
      reason: text(item.reason, `revocations[${index}].reason`, 1_000),
      revokedAt: text(item.revokedAt, `revocations[${index}].revokedAt`, 80),
    };
  });
  return { schemaVersion: 1, generatedAt: text(root.generatedAt, "generatedAt", 80), plugins, revocations };
}

async function boundedBytes(url: string, max: number): Promise<Uint8Array> {
  const response = await fetch(url, { cache: "no-store", redirect: "error" });
  if (!response.ok) throw new Error(`registry request failed (${response.status})`);
  const declared = Number(response.headers.get("content-length") ?? "0");
  if (declared > max) throw new Error("registry response is too large");
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > max) throw new Error("registry response is too large");
    return bytes;
  }
  const chunks: Uint8Array[] = [];
  let length = 0;
  const reader = response.body.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > max) throw new Error("registry response is too large");
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

async function boundedText(url: string, max: number): Promise<string> {
  return new TextDecoder("utf-8", { fatal: true }).decode(await boundedBytes(url, max));
}

async function verifiedIndex(indexJson: string, signature: string): Promise<RegistryIndex> {
  await backend().verifyPluginRegistry(indexJson, signature);
  return parseRegistryIndex(JSON.parse(indexJson));
}

export async function refreshCommunityRegistry(): Promise<void> {
  setRegistryState("loading");
  try {
    const [indexJson, signature] = await Promise.all([
      boundedText(COMMUNITY_REGISTRY_URL, MAX_INDEX_BYTES),
      boundedText(`${COMMUNITY_REGISTRY_URL}.sig`, 1_024),
    ]);
    const index = await verifiedIndex(indexJson, signature);
    await Promise.all([
      backend().setAppString("plugin-registry-index", indexJson),
      backend().setAppString("plugin-registry-signature", signature.trim()),
    ]);
    setCommunityPlugins(index.plugins);
    await pluginManager.applyRevocations(new Set(index.revocations.map((item) => `${item.id}@${item.version}`)));
    setRegistryState("ready");
  } catch {
    try {
      const [cached, signature] = await Promise.all([
        backend().getAppString("plugin-registry-index", ""),
        backend().getAppString("plugin-registry-signature", ""),
      ]);
      if (!cached || !signature) throw new Error("no verified registry cache");
      const index = await verifiedIndex(cached, signature);
      setCommunityPlugins(index.plugins);
      await pluginManager.applyRevocations(new Set(index.revocations.map((item) => `${item.id}@${item.version}`)));
      setRegistryState("offline");
    } catch {
      setCommunityPlugins([]);
      setRegistryState("invalid");
    }
  }
}

async function digestHex(bytes: Uint8Array): Promise<string> {
  const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function installCommunityPlugin(plugin: RegistryPlugin, version: RegistryVersion) {
  if (version.audit.status !== "passed") throw new Error("registry audit is not passing");
  const [manifestBytes, wasm] = await Promise.all([
    boundedBytes(version.manifestUrl, 64 * 1024),
    boundedBytes(version.wasmUrl, MAX_WASM_BYTES),
  ]);
  if ((await digestHex(manifestBytes)) !== version.manifestSha256) {
    throw new Error("plugin manifest digest does not match the signed registry");
  }
  if ((await digestHex(wasm)) !== version.sha256) throw new Error("plugin digest does not match the signed registry");
  const manifestText = new TextDecoder("utf-8", { fatal: true }).decode(manifestBytes);
  const manifest = parsePluginManifest(JSON.parse(manifestText));
  if (
    manifest.id !== plugin.id || manifest.version !== version.version || manifest.apiVersion !== version.apiVersion ||
    JSON.stringify(manifest.capabilities) !== JSON.stringify(version.capabilities) ||
    JSON.stringify(manifest.platforms) !== JSON.stringify(version.platforms)
  ) {
    throw new Error("plugin manifest does not match the signed registry metadata");
  }
  return pluginManager.install(manifest, wasm);
}
