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
import { THEME_API_VERSION, parseThemeManifest } from "../themes/manifest";
import { applyThemeRevocations, installThemePackage, themeVersionIsRevoked } from "../themes/manager";
import { applyTheme, selectedGalleryTheme } from "../themeGallery";

export const COMMUNITY_REGISTRY_URL =
  "https://raw.githubusercontent.com/martinkoutecky/tine-plugin-registry/main/index.json";
const MAX_INDEX_BYTES = 2 * 1024 * 1024;
const MAX_WASM_BYTES = 8 * 1024 * 1024;
const MAX_AUDIT_BYTES = 256 * 1024;
const NETWORK_READ_TIMEOUT_MS = 15_000;
const CACHE_LOAD_TIMEOUT_MS = 2_000;

export type RegistryAuditRisk = "low" | "review" | "elevated";
export type RegistryAuditDisposition = "publish" | "quarantine" | "reject";

export interface SafetyFinding {
  severity: "info" | "low" | "medium" | "high" | "critical";
  title: string;
  impact: string;
}

export interface PluginSafetyReport {
  automatedDisposition: RegistryAuditDisposition;
  deterministicStatus: "passed";
  risk: RegistryAuditRisk;
  checkedAt: string;
  sourceCommit: string;
  aiDisposition: "pass";
  uncertain: false;
  summary: string;
  findings: SafetyFinding[];
  areasReviewed: string[];
  manualApproval?: { by: string; note: string; approvedAt: string };
}

export interface RegistryVersion {
  version: string;
  apiVersion: string;
  platforms: Array<"desktop" | "android" | "ios">;
  capabilities: string[];
  sha256: string;
  manifestSha256: string;
  manifestUrl: string;
  wasmUrl: string;
  audit: {
    status: "passed";
    url: string;
    sha256: string;
    risk: RegistryAuditRisk;
    automatedDisposition: RegistryAuditDisposition;
    manualApproval: boolean;
    checkedAt: string;
  };
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

export interface RegistryThemeVersion {
  version: string;
  apiVersion: string;
  modes: Array<"light" | "dark">;
  manifestSha256: string;
  manifestUrl: string;
  audit: RegistryVersion["audit"];
  publishedAt: string;
}

export interface RegistryTheme {
  id: string;
  name: string;
  description: string;
  source: string;
  license: string;
  aiDevelopment: "none" | "assisted" | "primary";
  versions: RegistryThemeVersion[];
}

interface RegistryIndex {
  schemaVersion: 1;
  generatedAt: string;
  plugins: RegistryPlugin[];
  themes: RegistryTheme[];
  revocations: Array<{ id: string; version: string; severity: string; reason: string; revokedAt: string }>;
}

export interface VerifiedRegistrySnapshot {
  index: RegistryIndex;
  revoked: ReadonlySet<string>;
}

const [communityPlugins, setCommunityPlugins] = createSignal<RegistryPlugin[]>([]);
const [communityThemes, setCommunityThemes] = createSignal<RegistryTheme[]>([]);
const [registryState, setRegistryState] = createSignal<"idle" | "loading" | "ready" | "offline" | "invalid">("idle");
export { communityPlugins, communityThemes, registryState };

let hasVerifiedRegistry = false;
let refreshGeneration = 0;
let latestVerifiedGeneration = 0;
let liveApplyChain = Promise.resolve();

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

function oneOf<T extends string>(value: unknown, where: string, values: readonly T[]): T {
  if (typeof value !== "string" || !values.includes(value as T)) throw new Error(`${where} is invalid`);
  return value as T;
}

function digest(value: unknown, where: string): string {
  const result = text(value, where, 64);
  if (!/^[0-9a-f]{64}$/.test(result)) throw new Error(`${where} is invalid`);
  return result;
}

export function parseRegistryIndex(value: unknown): RegistryIndex {
  const root = object(value, "registry");
  knownKeys(root, "registry", ["schemaVersion", "generatedAt", "plugins", "themes", "revocations"]);
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
      const apiVersion = text(version.apiVersion, `${id}.apiVersion`, 32);
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
        if (typeof capability !== "string" || capability.length === 0 || capability.length > 80 ||
            (apiVersion === PLUGIN_API_VERSION && !PLUGIN_CAPABILITIES.includes(capability as PluginCapability))) {
          throw new Error(`${id} has an invalid capability`);
        }
        return capability as PluginCapability;
      });
      if (new Set(parsedCapabilities).size !== parsedCapabilities.length) throw new Error(`${id} has duplicate capabilities`);
      const audit = object(version.audit, `${id}.audit`);
      knownKeys(audit, `${id}.audit`, [
        "status", "url", "sha256", "risk", "automatedDisposition", "manualApproval", "checkedAt",
      ]);
      if (audit.status !== "passed") throw new Error(`${id} does not have a passing audit`);
      if (typeof audit.manualApproval !== "boolean") throw new Error(`${id} has invalid manual-approval metadata`);
      return {
        version: parsedVersion,
        apiVersion,
        platforms: parsedPlatforms,
        capabilities: parsedCapabilities,
        sha256,
        manifestSha256,
        manifestUrl: https(version.manifestUrl, `${id}.manifestUrl`),
        wasmUrl: https(version.wasmUrl, `${id}.wasmUrl`),
        audit: {
          status: "passed",
          url: https(audit.url, `${id}.audit.url`),
          sha256: digest(audit.sha256, `${id}.audit.sha256`),
          risk: oneOf(audit.risk, `${id}.audit.risk`, ["low", "review", "elevated"]),
          automatedDisposition: oneOf(
            audit.automatedDisposition,
            `${id}.audit.automatedDisposition`,
            ["publish", "quarantine", "reject"]
          ),
          manualApproval: audit.manualApproval,
          checkedAt: text(audit.checkedAt, `${id}.audit.checkedAt`, 80),
        },
        publishedAt: text(version.publishedAt, `${id}.publishedAt`, 80),
      };
    }).filter((version) => version.apiVersion === PLUGIN_API_VERSION);
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
  }).filter((plugin) => plugin.versions.length > 0);
  const themesValue = root.themes ?? [];
  if (!Array.isArray(themesValue)) throw new Error("registry themes are invalid");
  const themes = themesValue.map((candidate, themeIndex): RegistryTheme => {
    const item = object(candidate, `themes[${themeIndex}]`);
    knownKeys(item, `themes[${themeIndex}]`, ["id", "name", "description", "source", "license", "aiDevelopment", "versions"]);
    const id = text(item.id, `themes[${themeIndex}].id`, 64);
    if (!/^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$/.test(id) || !id.includes(".")) {
      throw new Error(`themes[${themeIndex}].id is invalid`);
    }
    if (ids.has(id)) throw new Error(`duplicate registry extension ${id}`);
    ids.add(id);
    if (!Array.isArray(item.versions) || item.versions.length === 0) throw new Error(`${id} has no versions`);
    const seenVersions = new Set<string>();
    const versions = item.versions.map((candidateVersion, versionIndex): RegistryThemeVersion => {
      const version = object(candidateVersion, `${id}.versions[${versionIndex}]`);
      knownKeys(version, `${id}.versions[${versionIndex}]`, [
        "version", "apiVersion", "modes", "manifestSha256", "manifestUrl", "audit", "publishedAt",
      ]);
      const parsedVersion = text(version.version, `${id}.version`, 64);
      if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/.test(parsedVersion)) {
        throw new Error(`${id} has an invalid version`);
      }
      if (seenVersions.has(parsedVersion)) throw new Error(`${id} has duplicate version ${parsedVersion}`);
      seenVersions.add(parsedVersion);
      const apiVersion = text(version.apiVersion, `${id}.apiVersion`, 32);
      if (!Array.isArray(version.modes) || version.modes.length === 0 ||
          version.modes.some((mode) => mode !== "light" && mode !== "dark") ||
          new Set(version.modes).size !== version.modes.length) {
        throw new Error(`${id} has invalid theme modes`);
      }
      const audit = object(version.audit, `${id}.audit`);
      knownKeys(audit, `${id}.audit`, [
        "status", "url", "sha256", "risk", "automatedDisposition", "manualApproval", "checkedAt",
      ]);
      if (audit.status !== "passed" || typeof audit.manualApproval !== "boolean") {
        throw new Error(`${id} does not have a passing theme audit`);
      }
      return {
        version: parsedVersion,
        apiVersion,
        modes: version.modes as Array<"light" | "dark">,
        manifestSha256: digest(version.manifestSha256, `${id}.manifestSha256`),
        manifestUrl: https(version.manifestUrl, `${id}.manifestUrl`),
        audit: {
          status: "passed",
          url: https(audit.url, `${id}.audit.url`),
          sha256: digest(audit.sha256, `${id}.audit.sha256`),
          risk: oneOf(audit.risk, `${id}.audit.risk`, ["low", "review", "elevated"]),
          automatedDisposition: oneOf(audit.automatedDisposition, `${id}.audit.automatedDisposition`, ["publish", "quarantine", "reject"]),
          manualApproval: audit.manualApproval,
          checkedAt: text(audit.checkedAt, `${id}.audit.checkedAt`, 80),
        },
        publishedAt: text(version.publishedAt, `${id}.publishedAt`, 80),
      };
    }).filter((version) => version.apiVersion === THEME_API_VERSION);
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
  }).filter((theme) => theme.versions.length > 0);
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
  return { schemaVersion: 1, generatedAt: text(root.generatedAt, "generatedAt", 80), plugins, themes, revocations };
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise<T>((resolve, reject) => {
    const aborted = () => reject(signal.reason ?? new Error("registry request timed out"));
    signal.addEventListener("abort", aborted, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", aborted);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", aborted);
        reject(error);
      }
    );
  });
}

async function boundedBytes(url: string, max: number, timeoutMs = NETWORK_READ_TIMEOUT_MS): Promise<Uint8Array> {
  const controller = new AbortController();
  const deadline = setTimeout(() => controller.abort(new Error("registry request timed out")), timeoutMs);
  try {
    const response = await abortable(fetch(url, {
      cache: "no-store",
      redirect: "error",
      signal: controller.signal,
    }), controller.signal);
    if (!response.ok) throw new Error(`registry request failed (${response.status})`);
    const declared = Number(response.headers.get("content-length") ?? "0");
    if (declared > max) throw new Error("registry response is too large");
    if (!response.body) {
      const bytes = new Uint8Array(await abortable(response.arrayBuffer(), controller.signal));
      if (bytes.byteLength > max) throw new Error("registry response is too large");
      return bytes;
    }
    const chunks: Uint8Array[] = [];
    let length = 0;
    const reader = response.body.getReader();
    try {
      while (true) {
        const { done, value } = await abortable(reader.read(), controller.signal);
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
  } finally {
    clearTimeout(deadline);
  }
}

async function boundedText(url: string, max: number, timeoutMs = NETWORK_READ_TIMEOUT_MS): Promise<string> {
  return new TextDecoder("utf-8", { fatal: true }).decode(await boundedBytes(url, max, timeoutMs));
}

async function verifiedIndex(indexJson: string, signature: string): Promise<RegistryIndex> {
  await backend().verifyPluginRegistry(indexJson, signature);
  return parseRegistryIndex(JSON.parse(indexJson));
}

function snapshot(index: RegistryIndex): VerifiedRegistrySnapshot {
  return {
    index,
    revoked: new Set(index.revocations.map((item) => `${item.id}@${item.version}`)),
  };
}

async function withDeadline<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let deadline: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_resolve, reject) => {
        deadline = setTimeout(() => reject(new Error("registry cache load timed out")), timeoutMs);
      }),
    ]);
  } finally {
    if (deadline !== undefined) clearTimeout(deadline);
  }
}

export async function loadVerifiedCachedRegistry(
  timeoutMs = CACHE_LOAD_TIMEOUT_MS
): Promise<VerifiedRegistrySnapshot | null> {
  try {
    return await withDeadline((async () => {
      const [cached, signature] = await Promise.all([
        backend().getAppString("plugin-registry-index", ""),
        backend().getAppString("plugin-registry-signature", ""),
      ]);
      if (!cached || !signature) throw new Error("no verified registry cache");
      return snapshot(await verifiedIndex(cached, signature));
    })(), timeoutMs);
  } catch {
    return null;
  }
}

export function seedCachedCommunityRegistry(cached: VerifiedRegistrySnapshot | null): ReadonlySet<string> {
  if (!cached) {
    hasVerifiedRegistry = false;
    setCommunityPlugins([]);
    setCommunityThemes([]);
    applyThemeRevocations(new Set());
    setRegistryState("invalid");
    return new Set();
  }
  hasVerifiedRegistry = true;
  setCommunityPlugins(cached.index.plugins);
  setCommunityThemes(cached.index.themes);
  applyThemeRevocations(cached.revoked);
  setRegistryState("offline");
  return cached.revoked;
}

async function applyLiveSnapshot(
  current: VerifiedRegistrySnapshot,
  generation: number,
  cache: { indexJson: string; signature: string }
): Promise<void> {
  liveApplyChain = liveApplyChain.catch(() => {}).then(async () => {
    if (generation !== latestVerifiedGeneration) return;
    await pluginManager.applyRevocations(current.revoked);
    if (generation !== latestVerifiedGeneration) return;
    setCommunityPlugins(current.index.plugins);
    setCommunityThemes(current.index.themes);
    applyThemeRevocations(current.revoked);
    applyTheme(selectedGalleryTheme());
    hasVerifiedRegistry = true;
    setRegistryState("ready");
    // Keep the durable pair inside the same accepted-generation queue as live
    // application. A stale response writes nothing; if a newer response verifies
    // while this write is already in flight, its queued write runs afterward.
    await Promise.all([
      backend().setAppString("plugin-registry-index", cache.indexJson),
      backend().setAppString("plugin-registry-signature", cache.signature.trim()),
    ]).catch(() => {});
  });
  await liveApplyChain;
}

export async function refreshCommunityRegistry(
  options: { timeoutMs?: number } = {}
): Promise<void> {
  const generation = ++refreshGeneration;
  setRegistryState("loading");
  try {
    const [indexJson, signature] = await Promise.all([
      boundedText(COMMUNITY_REGISTRY_URL, MAX_INDEX_BYTES, options.timeoutMs),
      boundedText(`${COMMUNITY_REGISTRY_URL}.sig`, 1_024, options.timeoutMs),
    ]);
    const current = snapshot(await verifiedIndex(indexJson, signature));
    latestVerifiedGeneration = Math.max(latestVerifiedGeneration, generation);
    await applyLiveSnapshot(current, generation, { indexJson, signature });
  } catch {
    // Cache verification and application are a separate startup phase. A live
    // timeout never clears or re-applies older state over a verified snapshot.
    if (generation === refreshGeneration) setRegistryState(hasVerifiedRegistry ? "offline" : "invalid");
  }
}

async function digestHex(bytes: Uint8Array): Promise<string> {
  const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function optionalManualApproval(value: unknown): PluginSafetyReport["manualApproval"] {
  if (value === undefined) return undefined;
  const approval = object(value, "manualApproval");
  return {
    by: text(approval.by, "manualApproval.by", 160),
    note: text(approval.note, "manualApproval.note", 2_000),
    approvedAt: text(approval.approvedAt, "manualApproval.approvedAt", 80),
  };
}

export function parseSafetyReport(
  value: unknown,
  plugin: RegistryPlugin,
  version: RegistryVersion
): PluginSafetyReport {
  const root = object(value, "safety report");
  if (root.format !== "tine-plugin-audit-result/v1") throw new Error("safety report format is incompatible");
  const submission = object(root.submission, "safety report submission");
  const submissionId = submission.schemaVersion === 2 && submission.kind === "plugin"
    ? submission.packageId
    : submission.pluginId;
  if (submissionId !== plugin.id || submission.version !== version.version) {
    throw new Error("safety report identity does not match the signed registry");
  }
  const sourceCommit = text(root.commitVerified, "commitVerified", 40);
  if (!/^[0-9a-f]{40}$/.test(sourceCommit) || submission.commit !== sourceCommit) {
    throw new Error("safety report source commit is invalid");
  }
  const checker = object(root.checker, "checker");
  const deterministicStatus = oneOf(checker.status, "checker.status", ["passed"]);
  const risk = oneOf(checker.risk, "checker.risk", ["low", "review", "elevated"]);
  const checkedAt = text(checker.checkedAt, "checker.checkedAt", 80);
  const automatedDisposition = oneOf(
    root.disposition,
    "disposition",
    ["publish", "quarantine", "reject"]
  );
  const ai = object(root.aiReview, "aiReview");
  const aiDisposition = oneOf(ai.disposition, "aiReview.disposition", ["pass"]);
  if (ai.uncertain !== false) throw new Error("safety report is uncertain");
  if (!Array.isArray(ai.findings) || ai.findings.length > 50 || !Array.isArray(ai.areasReviewed)) {
    throw new Error("safety report review details are invalid");
  }
  const findings = ai.findings.map((candidate, index): SafetyFinding => {
    const finding = object(candidate, `findings[${index}]`);
    return {
      severity: oneOf(finding.severity, `findings[${index}].severity`, ["info", "low", "medium", "high", "critical"]),
      title: text(finding.title, `findings[${index}].title`, 200),
      impact: text(finding.impact, `findings[${index}].impact`, 2_000),
    };
  });
  const areasReviewed = ai.areasReviewed.map((area, index) => text(area, `areasReviewed[${index}]`, 300));
  if (areasReviewed.length > 100) throw new Error("safety report has too many review areas");
  const manualApproval = optionalManualApproval(root.manualApproval);
  if (
    risk !== version.audit.risk ||
    checkedAt !== version.audit.checkedAt ||
    automatedDisposition !== version.audit.automatedDisposition ||
    !!manualApproval !== version.audit.manualApproval
  ) {
    throw new Error("safety report does not match its signed summary");
  }
  return {
    automatedDisposition,
    deterministicStatus,
    risk,
    checkedAt,
    sourceCommit,
    aiDisposition,
    uncertain: false,
    summary: text(ai.summary, "aiReview.summary", 4_000),
    findings,
    areasReviewed,
    ...(manualApproval ? { manualApproval } : {}),
  };
}

const safetyReportRequests = new Map<string, Promise<PluginSafetyReport>>();

export function loadSafetyReport(plugin: RegistryPlugin, version: RegistryVersion): Promise<PluginSafetyReport> {
  const key = version.audit.sha256;
  const existing = safetyReportRequests.get(key);
  if (existing) return existing;
  const request = (async () => {
    let bytes: Uint8Array;
    try {
      bytes = await boundedBytes(version.audit.url, MAX_AUDIT_BYTES);
      if ((await digestHex(bytes)) !== version.audit.sha256) throw new Error("safety report digest does not match the signed registry");
      const report = parseSafetyReport(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)), plugin, version);
      await backend().setAppString(`plugin-audit:${key}`, new TextDecoder().decode(bytes)).catch(() => {});
      return report;
    } catch (networkError) {
      const cached = await backend().getAppString(`plugin-audit:${key}`, "");
      if (!cached) throw networkError;
      bytes = new TextEncoder().encode(cached);
      if ((await digestHex(bytes)) !== version.audit.sha256) throw networkError;
      return parseSafetyReport(JSON.parse(cached), plugin, version);
    }
  })();
  safetyReportRequests.set(key, request);
  request.catch(() => safetyReportRequests.delete(key));
  return request;
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

export async function installCommunityTheme(theme: RegistryTheme, version: RegistryThemeVersion) {
  if (version.audit.status !== "passed") throw new Error("registry theme audit is not passing");
  if (themeVersionIsRevoked(`${theme.id}@${version.version}`)) {
    throw new Error("this theme version was revoked by the signed registry");
  }
  const manifestBytes = await boundedBytes(version.manifestUrl, 64 * 1024);
  if ((await digestHex(manifestBytes)) !== version.manifestSha256) {
    throw new Error("theme manifest digest does not match the signed registry");
  }
  const manifest = parseThemeManifest(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(manifestBytes)));
  const manifestModes = Object.keys(manifest.modes).sort();
  const registryModes = [...version.modes].sort();
  if (
    manifest.id !== theme.id || manifest.version !== version.version || manifest.apiVersion !== version.apiVersion ||
    JSON.stringify(manifestModes) !== JSON.stringify(registryModes)
  ) {
    throw new Error("theme manifest does not match the signed registry metadata");
  }
  return installThemePackage(manifest);
}
