// The diff/bench orchestrator — drives the whole "Help improve Tine" run:
// enumerate graph files (backend), parse each with lsdoc (main thread) and mldoc
// (worker), compare canonical projections, and for every mismatch re-verify in a
// FRESH mldoc realm, shrink to the smallest divergent range, and produce an
// anonymized-and-re-verified snippet. This is the faithful in-app analog of
// graph-check.mjs's runDiff/runBench; the pure logic lives in the engine modules.
import { backend } from "../../backend";
import { readOwned, type Owned, type Owner } from "../../owned";
import type { GraphSourceFile } from "../../backend";
import { MldocClient, type Format, type Projection } from "./mldoc-client";
import { lsdocDocumentAvailable, lsdocVersion, parseLsdocDocument } from "./lsdoc-document";
import { projectionKey } from "./projection";
import { lineNumberForOffset, minimize, toBytes } from "./minimize";
import { anonymizeAndVerify, anonymizeSourceRel } from "./anonymize";
import { benchFromResults, summarizeBenchRuns, type BenchRun, type BenchSummary } from "./bench";
import {
  isMldocBacktickStateArtifact,
  mldocBacktickArtifactSourceSpan,
  shouldQuarantineMldocBacktickStateArtifact,
} from "./oracle-artifacts";

export interface DiffOptions {
  mode: "diff" | "bench" | "both";
  includeJournals: boolean;
  fast: boolean; // scan with a warm worker only (non-authoritative absences)
  timeoutMs: number;
}

export type Finding =
  | {
      type: "divergence";
      rel: string;
      lineStart: number;
      lineEnd: number;
      contextDependent: boolean;
      anonymized:
        | { ok: true; tier: string; input: string; lsdocKey: string; mldocKey: string }
        | { ok: false };
    }
  | { type: "mldoc-failure"; rel: string; status: string; detail: string }
  | { type: "unstable-divergence"; rel: string }
  | {
      type: "intentional-divergence";
      rel: string;
      lineStart: number;
      lineEnd: number;
      kind: "md-nested-dollar-latex";
      detail: string;
    }
  | {
      type: "mldoc-oracle-artifact";
      rel: string;
      lineStart: number;
      lineEnd: number;
      detail: string;
    };

export interface DiffReport {
  tineVersion: string;
  lsdocVersion: string;
  stats: { files: number; totalBytes: number };
  lsdocAvailable: boolean;
  bench?: { lsdoc: BenchSummary | null; mldoc: BenchSummary };
  findings?: Finding[];
}

export interface ProgressEvent {
  phase: "scan" | "verify" | "bench";
  done: number;
  total: number;
  current?: string;
}

const BENCH_RUNS = 3; // best-of-3, like graph-check

interface PairResult {
  ok: boolean;
  diverges: boolean;
  lsdocProjection?: Projection;
  mldocProjection?: Projection;
}

/** Read source files for the whole graph, compare lsdoc and mldoc projections
 * as requested, and report progress. Cost and memory grow with graph file
 * count and bytes; parser work and minimization may add more. Returns stale
 * if ownership retires at an owned result boundary. Backend, parser or progress
 * callback errors reject. */
export async function runComparison(
  opts: DiffOptions,
  onProgress: (e: ProgressEvent) => void,
  owner: Owner,
): Promise<Owned<DiffReport>> {
  const tineVersion = await currentTineVersion();
  // Screenshot/dev hook: a preloaded fixture lets the harness render the panel's
  // populated state without a live mldoc+lsdoc run. Never set in a real build.
  const fixture = (globalThis as unknown as { __tineDiffFixture?: DiffReport }).__tineDiffFixture;
  if (fixture) return owner()
    ? { kind: "current", value: { ...fixture, tineVersion: fixture.tineVersion || tineVersion } }
    : { kind: "stale" };

  const loaded = await readOwned(owner, backend().graphSourceFiles(opts.includeJournals));
  if (loaded.kind === "stale") return loaded;
  const files = loaded.value;
  const stats = { files: files.length, totalBytes: files.reduce((n, f) => n + f.bytes, 0) };
  const lsdocAvailable = lsdocDocumentAvailable();
  const parserVersion = lsdocVersion();
  const client = new MldocClient();

  // Fresh (authoritative) both-parser parse — feeds re-verify, minimize, anon.
  const parseBothFresh = async (text: string, format: Format): Promise<PairResult> => {
    if (!lsdocAvailable) return { ok: false, diverges: false };
    let lsdocProjection: Projection;
    try {
      lsdocProjection = parseLsdocDocument(text, format === "org");
    } catch {
      return { ok: false, diverges: false };
    }
    const m = await client.parseFresh(text, format, opts.timeoutMs);
    if (!m.ok) return { ok: false, diverges: false };
    return {
      ok: true,
      diverges: projectionKey(lsdocProjection) !== projectionKey(m.projection),
      lsdocProjection,
      mldocProjection: m.projection,
    };
  };

  try {
    let bench: DiffReport["bench"];
    let findings: Finding[] | undefined;

    if (opts.mode === "bench" || opts.mode === "both") {
      bench = await runBench(client, files, opts, onProgress);
    }
    if (opts.mode === "diff" || opts.mode === "both") {
      findings = lsdocAvailable ? await runDiff(client, files, opts, parseBothFresh, onProgress) : [];
    }

    return owner()
      ? { kind: "current", value: { tineVersion, lsdocVersion: parserVersion, stats, lsdocAvailable, bench, findings } }
      : { kind: "stale" };
  } finally {
    client.dispose();
  }
}

async function runDiff(
  client: MldocClient,
  files: GraphSourceFile[],
  opts: DiffOptions,
  parseBothFresh: (text: string, format: Format) => Promise<PairResult>,
  onProgress: (e: ProgressEvent) => void,
): Promise<Finding[]> {
  const anonymousRels = files.map((f, i) => anonymizeSourceRel(f.rel, i));
  // Stage 1 — fast scan (warm worker) to find candidate mismatches.
  const candidates: { file: GraphSourceFile; rel: string }[] = [];
  const findings: Finding[] = [];
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    onProgress({ phase: "scan", done: i, total: files.length, current: anonymousRels[i] });
    let lsdocKey: string;
    try {
      lsdocKey = projectionKey(parseLsdocDocument(f.text, f.format === "org"));
    } catch (e) {
      findings.push({ type: "mldoc-failure", rel: anonymousRels[i], status: "lsdoc-error", detail: String(e).split("\n")[0] });
      continue;
    }
    const m = await client.parseWarm(f.text, f.format, opts.timeoutMs);
    if (!m.ok) {
      findings.push({ type: "mldoc-failure", rel: anonymousRels[i], status: m.status, detail: m.detail });
      continue;
    }
    if (lsdocKey !== projectionKey(m.projection)) candidates.push({ file: f, rel: anonymousRels[i] });
  }

  // Stage 2 — re-verify each candidate in fresh realms, minimize, anonymize.
  for (let i = 0; i < candidates.length; i++) {
    const { file: f, rel } = candidates[i];
    onProgress({ phase: "verify", done: i, total: candidates.length, current: rel });
    const original = await parseBothFresh(f.text, f.format);
    if (!original.ok || !original.diverges) {
      findings.push({ type: "unstable-divergence", rel });
      continue;
    }
    const buf = toBytes(f.text);
    const min = await minimize(buf, f.format, (t, fmt) => parseBothFresh(t, fmt));
    // `contextDependent` means the minimizer could not reproduce the mismatch in
    // any independently parsed range; every such probe used parseFresh and thus
    // a brand-new mldoc realm. Suppress only the exact issue #82 one-backtick
    // Plain/Code ownership shift. Other context-sensitive differences remain
    // actionable divergences.
    const artifactSpan = original.lsdocProjection && original.mldocProjection
      ? mldocBacktickArtifactSourceSpan(original.lsdocProjection, original.mldocProjection)
      : null;
    if (
      original.lsdocProjection
      && original.mldocProjection
      && artifactSpan
      && shouldQuarantineMldocBacktickStateArtifact(
        min.contextDependent,
        original.lsdocProjection,
        original.mldocProjection,
      )
    ) {
      const isolatedText = new TextDecoder().decode(buf.subarray(artifactSpan[0], artifactSpan[1]));
      const isolated = await parseBothFresh(isolatedText, f.format);
      if (isolated.ok && !isolated.diverges) {
        findings.push({
          type: "mldoc-oracle-artifact",
          rel,
          lineStart: lineNumberForOffset(buf, artifactSpan[0]),
          lineEnd: lineNumberForOffset(buf, Math.max(artifactSpan[0], artifactSpan[1] - 1)),
          detail: "suppressed: mldoc leaked failed double-backtick parser state; a fresh isolated-block parse agrees",
        });
        continue;
      }
    }
    const minimizedOriginal = await parseBothFresh(min.input, f.format);
    if (
      minimizedOriginal.ok
      && minimizedOriginal.diverges
      && minimizedOriginal.lsdocProjection
      && await isIntentionalNestedDollarDivergence(
        min.input,
        f.format,
        minimizedOriginal.lsdocProjection,
        parseBothFresh,
      )
    ) {
      findings.push({
        type: "intentional-divergence",
        rel,
        lineStart: min.lineStart,
        lineEnd: min.lineEnd,
        kind: "md-nested-dollar-latex",
        detail: "suppressed: Tine intentionally preserves dollar math inside Markdown emphasis",
      });
      continue;
    }
    const anon = minimizedOriginal.ok && minimizedOriginal.diverges
      ? await anonymizeAndVerify<Projection>(
          min.input,
          (candidate) => parseBothFresh(candidate, f.format),
          (parsed) => !(
            parsed.lsdocProjection
            && parsed.mldocProjection
            && isMldocBacktickStateArtifact(parsed.lsdocProjection, parsed.mldocProjection)
          ),
          minimizedOriginal,
        )
      : { ok: false };
    findings.push({
      type: "divergence",
      rel,
      lineStart: min.lineStart,
      lineEnd: min.lineEnd,
      contextDependent: min.contextDependent,
      anonymized: anon.ok
        ? {
            ok: true,
            tier: anon.tier!,
            input: anon.input!,
            lsdocKey: projectionKey(anon.lsdocProjection),
            mldocKey: projectionKey(anon.mldocProjection),
          }
        : { ok: false },
    });
  }
  return findings;
}

type JsonObject = Record<string, unknown>;
type NestedDollarCandidate = {
  path: (string | number)[];
  parentPath: (string | number)[];
  start: number;
  end: number;
};

function jsonObject(value: unknown): JsonObject | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function valueAtPath(root: unknown, path: (string | number)[]): unknown {
  let value = root;
  for (const part of path) {
    if (Array.isArray(value) && typeof part === "number") value = value[part];
    else value = jsonObject(value)?.[String(part)];
  }
  return value;
}

function nestedDollarCandidates(root: unknown, source: Uint8Array): NestedDollarCandidate[] | null {
  const candidates: NestedDollarCandidate[] = [];
  let invalid = false;
  const walk = (value: unknown, path: (string | number)[]) => {
    if (Array.isArray(value)) {
      value.forEach((child, index) => walk(child, [...path, index]));
      return;
    }
    const object = jsonObject(value);
    if (!object) return;
    if (object.k === "emphasis" && Array.isArray(object.children)) {
      object.children.forEach((child, index) => {
        const node = jsonObject(child);
        if (node?.k === "latex") {
          const span = node.span;
          const delimiter = node.mode === "Inline" ? "$" : node.mode === "Displayed" ? "$$" : null;
          if (
            !delimiter
            || typeof node.body !== "string"
            || !Array.isArray(span)
            || span.length !== 2
            || !span.every(Number.isInteger)
          ) {
            invalid = true;
          } else {
            const [start, end] = span as [number, number];
            const expected = new TextEncoder().encode(`${delimiter}${node.body}${delimiter}`);
            const actual = source.slice(start, end);
            if (
              start < 0
              || start >= end
              || end > source.length
              || actual.length !== expected.length
              || actual.some((byte, offset) => byte !== expected[offset])
            ) invalid = true;
            else candidates.push({ path: [...path, "children", index], parentPath: [...path, "children"], start, end });
          }
        }
        walk(child, [...path, "children", index]);
      });
      for (const [key, child] of Object.entries(object)) {
        if (key !== "children") walk(child, [...path, key]);
      }
      return;
    }
    for (const [key, child] of Object.entries(object)) walk(child, [...path, key]);
  };
  walk(root, []);
  candidates.sort((a, b) => a.start - b.start);
  if (invalid || candidates.length === 0) return null;
  if (candidates.some((candidate, index) => index > 0 && candidate.start < candidates[index - 1].end)) return null;
  return candidates;
}

function coalescePlainChildren(children: unknown[]): unknown[] {
  const result: unknown[] = [];
  for (const child of children) {
    const object = jsonObject(child);
    const previous = jsonObject(result[result.length - 1]);
    if (object?.k === "plain" && previous?.k === "plain" && typeof object.text === "string" && typeof previous.text === "string") {
      previous.text += object.text;
    } else result.push(child);
  }
  return result;
}

function transformedNestedDollarProjection(
  original: Projection,
  candidates: NestedDollarCandidate[],
): Projection | null {
  const transformed = structuredClone(original);
  const parents = new Map<string, (string | number)[]>();
  for (const candidate of candidates) {
    const parent = valueAtPath(transformed, candidate.parentPath);
    const index = candidate.path[candidate.path.length - 1];
    if (!Array.isArray(parent) || typeof index !== "number") return null;
    const node = jsonObject(parent[index]);
    if (node?.k !== "latex") return null;
    parent[index] = { k: "plain", text: ",".repeat(candidate.end - candidate.start) };
    parents.set(JSON.stringify(candidate.parentPath), candidate.parentPath);
  }
  for (const path of parents.values()) {
    const children = valueAtPath(transformed, path);
    const owner = jsonObject(valueAtPath(transformed, path.slice(0, -1)));
    const key = path[path.length - 1];
    if (!Array.isArray(children) || !owner || typeof key !== "string") return null;
    owner[key] = coalescePlainChildren(children);
  }
  return transformed;
}

export async function isIntentionalNestedDollarDivergence(
  input: string,
  format: Format,
  lsdocOriginal: Projection,
  parseBothFresh: (text: string, format: Format) => Promise<PairResult>,
): Promise<boolean> {
  if (format !== "md") return false;
  const source = new TextEncoder().encode(input);
  const candidates = nestedDollarCandidates(lsdocOriginal, source);
  if (!candidates) return false;
  for (const candidate of candidates) {
    const standalone = new TextDecoder().decode(source.slice(candidate.start, candidate.end));
    const parsed = await parseBothFresh(standalone, format);
    if (!parsed.ok || parsed.diverges) return false;
  }
  const masked = source.slice();
  for (const candidate of candidates) masked.fill(",".charCodeAt(0), candidate.start, candidate.end);
  const maskedPair = await parseBothFresh(new TextDecoder().decode(masked), format);
  if (!maskedPair.ok || maskedPair.diverges || !maskedPair.lsdocProjection) return false;
  const transformed = transformedNestedDollarProjection(lsdocOriginal, candidates);
  return !!transformed && projectionKey(transformed) === projectionKey(maskedPair.lsdocProjection);
}


async function currentTineVersion(): Promise<string> {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch {
    return "unknown";
  }
}

async function runBench(
  client: MldocClient,
  files: GraphSourceFile[],
  opts: DiffOptions,
  onProgress: (e: ProgressEvent) => void,
): Promise<DiffReport["bench"]> {
  const lsdocAvailable = lsdocDocumentAvailable();
  const idFiles = files.map((f, i) => ({ id: `b${i}`, rel: anonymizeSourceRel(f.rel, i) }));

  // mldoc: warm worker timing (reused; matches graph-check's benchMldoc).
  const mldocRuns: BenchRun[] = [];
  for (let run = 0; run < BENCH_RUNS; run++) {
    const results = new Map<string, { ok: boolean; parseMicros?: number; status?: string; detail?: string; overTimeout?: boolean }>();
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      onProgress({ phase: "bench", done: run * files.length + i, total: BENCH_RUNS * files.length, current: `mldoc ${idFiles[i].rel}` });
      const m = await client.parseWarm(f.text, f.format, opts.timeoutMs);
      results.set(idFiles[i].id, m.ok ? { ok: true, parseMicros: m.parseMicros } : { ok: false, status: m.status, detail: m.detail });
    }
    mldocRuns.push(benchFromResults(idFiles, results));
  }

  // lsdoc: main-thread timing (only if the whole-file parser is wired).
  let lsdocSummary: BenchSummary | null = null;
  if (lsdocAvailable) {
    const lsdocRuns: BenchRun[] = [];
    for (let run = 0; run < BENCH_RUNS; run++) {
      const results = new Map<string, { ok: boolean; parseMicros?: number; status?: string; detail?: string }>();
      for (const [i, f] of files.entries()) {
        const start = performance.now();
        try {
          parseLsdocDocument(f.text, f.format === "org");
          results.set(idFiles[i].id, { ok: true, parseMicros: Math.round((performance.now() - start) * 1000) });
        } catch (e) {
          results.set(idFiles[i].id, { ok: false, status: "lsdoc-error", detail: String(e).split("\n")[0] });
        }
      }
      lsdocRuns.push(benchFromResults(idFiles, results));
    }
    lsdocSummary = summarizeBenchRuns(lsdocRuns);
  }

  return { lsdoc: lsdocSummary, mldoc: summarizeBenchRuns(mldocRuns) };
}
