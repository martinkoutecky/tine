import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  AdoptionArchivedError,
  AssetTooLargeError,
  BackendError,
  DirectSaveFailureError,
  ManagedActorRefusalError,
  ManagedGraphMismatchError,
  OperationCancelledError,
  SaveConflictError,
  SharedFrontierMismatchError,
  SparseShutdownRefusedError,
  SyncDataUnavailableError,
  classifyNativeCallError,
} from "./backend";

const ROOT = fileURLToPath(new URL(".", import.meta.url));
const source = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

function hasStringErrorResult(text: string): boolean {
  let rest = text;
  while (true) {
    const start = rest.indexOf("Result<");
    if (start < 0) return false;
    rest = rest.slice(start + "Result<".length);
    let depth = 1;
    let comma = -1;
    let end = -1;
    for (let index = 0; index < rest.length; index += 1) {
      if (rest[index] === "<") depth += 1;
      else if (rest[index] === ">") {
        depth -= 1;
        if (depth === 0) { end = index; break; }
      } else if (rest[index] === "," && depth === 1) comma = index;
    }
    if (end < 0) return false;
    if (comma >= 0 && rest.slice(comma + 1, end).trim() === "String") return true;
    rest = rest.slice(end + 1);
  }
}

function enclosingRustSymbol(text: string, offset: number): string | null {
  const prefix = text.slice(0, offset);
  const functions = [...prefix.matchAll(/\b(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]+"\s+)?fn\s+([A-Za-z_$][\w$]*)/g)];
  return functions.at(-1)?.[1] ?? null;
}

function withoutRustTestModules(text: string): string {
  const chars = [...text];
  const module = /#\[cfg\(test\)\]\s*(?:pub\(crate\)\s+)?mod\s+\w+\s*\{/g;
  for (const match of text.matchAll(module)) {
    const open = match.index + match[0].lastIndexOf("{");
    let depth = 0;
    let end = open;
    for (; end < text.length; end += 1) {
      if (text[end] === "{") depth += 1;
      else if (text[end] === "}") {
        depth -= 1;
        if (depth === 0) { end += 1; break; }
      }
    }
    for (let index = match.index; index < end; index += 1) chars[index] = " ";
  }
  return chars.join("");
}

interface ClassifierSite {
  file: string;
  line: number;
  class: string;
  why: string;
}

const ERROR_STRING_CLASSIFIER_ALLOWLIST: readonly ClassifierSite[] = [
  {
    file: "components/Macro.tsx",
    line: 458,
    class: "bounded-result-code",
    why: "the query boundary's result-too-large prefix is a bounded wire code, not prose",
  },
  {
    file: "lib/referenceLoadError.ts",
    line: 26,
    class: "bounded-result-code",
    why: "the references boundary's result-too-large prefix is a bounded wire code, not prose",
  },
];

function sourceFiles(dir: string, files: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      if (full.endsWith(join("lsdoc-diff", "vendor"))) continue;
      sourceFiles(full, files);
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry) && entry !== "backend.ts") {
      files.push(full);
    }
  }
  return files;
}

function errorStringClassifierSites(): Omit<ClassifierSite, "class" | "why">[] {
  const sites: Omit<ClassifierSite, "class" | "why">[] = [];
  for (const file of sourceFiles(ROOT)) {
    const text = readFileSync(file, "utf8");
    for (const [index, line] of text.split("\n").entries()) {
      if (
        /String\((?:error|err|e)\).*\.(?:includes|match|startsWith)\(/.test(line)
        || /\b(?:detail|message)\.(?:includes|match|startsWith)\(/.test(line)
        || /\.(?:exec|test)\((?:message|detail)\)/.test(line)
      ) {
        sites.push({ file: relative(ROOT, file).replaceAll("\\", "/"), line: index + 1 });
      }
    }

    const lines = text.split("\n");
    const functions: { name: string; start: number; end: number; parameter: string | null }[] = [];
    for (let index = 0; index < lines.length; index += 1) {
      const signature = lines[index].match(
        /(?:function\s+([A-Za-z_$][\w$]*)|(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?)\s*\([^)]*\)\s*(?::[^={]+)?(?:=>\s*)?\{/,
      );
      if (!signature) continue;
      const parameter = lines[index].match(/\b(message|text)\s*:\s*string\b/)?.[1] ?? null;
      let depth = 0;
      let end = index;
      for (; end < lines.length; end += 1) {
        depth += (lines[end].match(/\{/g) ?? []).length;
        depth -= (lines[end].match(/\}/g) ?? []).length;
        if (depth === 0) break;
      }
      functions.push({ name: signature[1] ?? signature[2], start: index, end, parameter });
      index = end;
    }

    const helperNames = new Set<string>();
    for (const fn of functions) {
      if (!fn.parameter || !/(?:error|failure|classif)/i.test(fn.name)) continue;
      const aliases = new Set([fn.parameter]);
      let firstDerivedInput: number | null = null;
      let classifierLine: number | null = null;
      for (let index = fn.start + 1; index <= fn.end; index += 1) {
        const line = lines[index];
        const derived = line.match(/\b(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*([A-Za-z_$][\w$]*)\.(?:slice|substring|replace|trim)\b/);
        if (derived && aliases.has(derived[2])) {
          aliases.add(derived[1]);
          firstDerivedInput ??= index;
        }
        const classifiesAlias = [...aliases].some((alias) =>
          new RegExp(`\\b${alias}\\.(?:includes|match|startsWith)\\(`).test(line)
          || new RegExp(`\\.(?:exec|test)\\(${alias}\\)`).test(line),
        );
        if (classifiesAlias) {
          classifierLine = firstDerivedInput ?? index;
          break;
        }
      }
      if (classifierLine !== null) {
        helperNames.add(fn.name);
        sites.push({
          file: relative(ROOT, file).replaceAll("\\", "/"),
          line: classifierLine + 1,
        });
      }
    }

    for (const fn of functions) {
      if (fn.parameter) continue;
      const body = lines.slice(fn.start, fn.end + 1).join("\n");
      const convertsErrorToText = /\b(?:message|text)\s*=\s*String\((?:error|err|e)\)/.test(body);
      const delegatesToHelper = [...helperNames].some((name) =>
        new RegExp(`\\b${name}\\((?:message|text)\\)`).test(body),
      );
      if (convertsErrorToText && delegatesToHelper) {
        sites.push({ file: relative(ROOT, file).replaceAll("\\", "/"), line: fn.start + 1 });
      }
    }
  }
  return sites
    .filter((site, index, all) =>
      all.findIndex((candidate) => candidate.file === site.file && candidate.line === site.line) === index,
    )
    .sort((left, right) => left.file.localeCompare(right.file) || left.line - right.line);
}

describe("I-9/I-11 typed backend error boundary", () => {
  it("classifies every tagged native payload once at the backend funnel", () => {
    const cases: [string, new (...args: never[]) => BackendError][] = [
      ["sync-data-unavailable", SyncDataUnavailableError],
      ["managed-graph-mismatch", ManagedGraphMismatchError],
      ["shared-frontier-mismatch", SharedFrontierMismatchError],
      ["adoption-archived", AdoptionArchivedError],
      ["sparse-shutdown-refused", SparseShutdownRefusedError],
      ["asset-too-large", AssetTooLargeError],
      ["operation-cancelled", OperationCancelledError],
    ];
    for (const [kind, Type] of cases) {
      const classified = classifyNativeCallError(JSON.stringify({ kind }));
      expect(classified).toBeInstanceOf(Type);
    }
    const actor = classifyNativeCallError(JSON.stringify({
      kind: "managed-actor-refusal",
      reason_code: "trusted_local.append_outcome_unknown",
    }));
    expect(actor).toBeInstanceOf(ManagedActorRefusalError);
    expect(actor).toMatchObject({ reasonCode: "trusted_local.append_outcome_unknown" });

    const direct = classifyNativeCallError(JSON.stringify({
      kind: "direct-save-failure",
      reason_code: "precheck.symlink",
      detail: { io_error_kind: "InvalidInput" },
    }));
    expect(direct).toBeInstanceOf(DirectSaveFailureError);
    expect(direct).toMatchObject({ reasonCode: "precheck.symlink", ioErrorKind: "InvalidInput" });

    const conflict = classifyNativeCallError(JSON.stringify({
      kind: "save-conflict",
      reason_code: "conflict.base_rev",
      detail: { io_error_kind: "AlreadyExists", epoch: 23 },
    }));
    expect(conflict).toBeInstanceOf(SaveConflictError);
    expect(conflict).toMatchObject({ reasonCode: "conflict.base_rev", epoch: 23 });

    // The one kind with a typed detail object: bounded counts and paths are
    // validated field by field; a malformed detail degrades to no detail.
    const mismatch = classifyNativeCallError(JSON.stringify({
      kind: "shared-frontier-mismatch",
      detail: {
        local_pages: 2, shared_pages: 2, local_only: 1, shared_only: 0, changed: 1, omitted: 0,
        paths: [
          { path: "notes/local-only.md", side: "local-only" },
          { path: "notes/changed.md", side: "changed", categories: ["outline"] },
        ],
      },
    }));
    expect(mismatch).toBeInstanceOf(SharedFrontierMismatchError);
    expect(mismatch).toMatchObject({
      detail: {
        localOnly: 1,
        paths: [
          { path: "notes/local-only.md", side: "local-only", categories: [] },
          { path: "notes/changed.md", side: "changed", categories: ["outline"] },
        ],
      },
    });
    const malformed = classifyNativeCallError(JSON.stringify({
      kind: "shared-frontier-mismatch",
      detail: { local_pages: 1, paths: [{ path: 7, side: "elsewhere" }] },
    }));
    expect(malformed).toBeInstanceOf(SharedFrontierMismatchError);
    expect(malformed).toMatchObject({ detail: null });
  });

  it("has no prose-parsing classifier outside the one funnel", () => {
    expect(
      errorStringClassifierSites(),
      "I-9: error classification must use src/backend.ts SaveConflictError/classifyTaggedBackendError; helper indirection may not restore prose parsing",
    ).toEqual(
      ERROR_STRING_CLASSIFIER_ALLOWLIST.map(({ file, line }) => ({ file, line })),
    );
  });

  it("keeps phase-B legacy literals compatible with the frontend funnel", () => {
    expect(classifyNativeCallError('{"kind":"operation-cancelled"}')).toBeInstanceOf(OperationCancelledError);
    expect(classifyNativeCallError('{"kind":"sync-data-unavailable"}')).toBeInstanceOf(SyncDataUnavailableError);
    expect(classifyNativeCallError('{"kind":"adoption-archived"}')).toBeInstanceOf(AdoptionArchivedError);
    for (const literal of [
      "denied", "asset not found: worker", "asset not found: tauri", "json failure",
      "plugin failure", "clipboard failure", "platform failure", "graph verification failure",
      "graph failure", "sync runtime failure", "settings failure", "diagnostic failure",
      "backup failure", "phase-B prose",
    ]) expect(classifyNativeCallError(literal)).toBe(literal);
  });

  it("pins the Rust typed boundaries and the living contract", () => {
    const wire = source("crates/tine-core/src/oplog/wire.rs");
    const runtime = source("crates/tine-core/src/sync_runtime.rs");
    const model = source("crates/tine-core/src/model.rs");
    const contract = source("docs/contracts/typed-errors.md");
    expect(wire).toContain("Io(std::io::ErrorKind)");
    expect(wire).not.toMatch(/ScenarioError::Io\([^)]*(?:to_string|format!)/s);
    const directClassifier = model.slice(
      model.indexOf("pub fn direct_save_conflict_epoch"),
      model.indexOf("fn initial_shadow_limit_error"),
    );
    expect(directClassifier).toContain("downcast_ref::<DirectSaveError>()");
    expect(directClassifier).not.toMatch(/(?:to_string|contains|starts_with)\s*\(/);
    // Clean-open failures stay typed until the single OpenRefused projection.
    expect(runtime.match(/map_err\(display\)/g) ?? []).toHaveLength(0);
    expect(runtime.match(/fn display\(/g) ?? []).toHaveLength(0);
    expect(contract).toContain("10 BackendError subclasses");
    expect(contract).toContain("Core-only clean-open boundary");
    expect(contract).not.toContain("item 3 checkpoint");
    expect(contract).toContain("TauriBackend.call");

    const commandError = source("src-tauri/src/command_error.rs");
    const commands = source("src-tauri/src/commands.rs");
    const state = source("src-tauri/src/state.rs");
    const parity = source("src-tauri/src/backend_command_parity.rs");
    expect(commandError).toContain("impl Serialize for CommandError");
    expect(commandError).toContain("serializer.serialize_str(&self.wire())");
    expect(commandError).not.toMatch(/impl From<(?:String|&str)> for CommandError/);
    expect(commands).not.toMatch(/map_err\(\|\w+\| \w+\.to_string\(\)\)/);
    expect(state).not.toMatch(/map_err\(\|\w+\| \w+\.to_string\(\)\)/);
    expect(contract).toContain("## `CommandError` boundary");
    expect(contract).toContain("The phase-A syntactic census is 116 production sites");

    const quitFixtures = commands.slice(
      commands.indexOf("mod prepare_tine_quit_tests"),
      commands.indexOf("pub(crate) fn read_local_image"),
    );
    const proseSites = (commands.match(/CommandError::prose/g) ?? []).length
      - (quitFixtures.match(/CommandError::prose/g) ?? []).length
      + (state.match(/CommandError::prose/g) ?? []).length;
    // 116, not 113: P0-rust's three new managed query commands each add one
    // wrong-reply arm, the category the contract's Prose census already records
    // as having no typed source. The ratchet retires legacy untyped WORDING; it
    // is not a cap on the managed-command surface. See docs/contracts/typed-errors.md.
    expect(proseSites).toBe(116);

    const phaseB = parity.slice(
      parity.indexOf("const PHASE_B_COMMANDS"),
      parity.indexOf("const INFALLIBLE"),
    );
    const phaseBRows = [...phaseB.matchAll(/\("([^"]+\.rs)", "([^"]+)"\)/g)];
    expect(phaseBRows.length).toBeGreaterThan(50);
    expect(contract).toContain("Every fallible command registered for desktop, Android, or iOS");

    for (const heading of ["### Conversion table", "### `Prose` census"]) {
      const start = contract.indexOf(heading);
      const end = contract.indexOf("\n##", start + heading.length);
      const section = contract.slice(start, end < 0 ? undefined : end);
      const rows = section.split("\n").filter((line) => line.startsWith("|") && !line.includes("---"));
      expect(rows.length, `${heading} must retain a header and checked rows`).toBeGreaterThan(1);
      for (const row of rows) expect(row.split("|").length).toBe(7);
    }

    const cleanEnum = runtime.slice(
      runtime.indexOf("pub(crate) enum CleanOpenError"),
      runtime.indexOf("impl CleanOpenError"),
    );
    expect(cleanEnum).not.toMatch(/\bString\b/);
    const cleanImpl = runtime.slice(
      runtime.indexOf("impl CleanOpenError"),
      runtime.indexOf("impl fmt::Display for CleanOpenError"),
    );
    const cleanCodes = [...cleanImpl.matchAll(/"(clean_open\.[a-z_]+)"/g)]
      .map((match) => match[1]);
    expect(cleanCodes).toHaveLength(16);
    expect(new Set(cleanCodes).size).toBe(16);
    for (const code of cleanCodes) expect(contract).toContain(code);
    expect(runtime.match(/fn clean_open_error_detail\(/g) ?? []).toHaveLength(1);
    expect(runtime).toContain('tagged_backend_error("clean-open", Some(error.reason_code()))');

    const directImpl = model.slice(
      model.indexOf("impl DirectSaveFailureCode"),
      model.indexOf("/// Typed inner error", model.indexOf("impl DirectSaveFailureCode")),
    );
    const directCodes = [...directImpl.matchAll(/"((?:precheck|identity|conflict|conflict_retry|conflict_authority)\.[a-z_]+|unknown)"/g)]
      .map((match) => match[1]);
    expect(directCodes).toHaveLength(36);

    const managedImpl = runtime.slice(
      runtime.indexOf("impl SyncEditorRefusalCode"),
      runtime.indexOf("impl fmt::Display for SyncEditorRefusalCode"),
    );
    const managedCodes = [...managedImpl.matchAll(/"([a-z][a-z_]*(?:\.[a-z][a-z_]*)+)"/g)]
      .map((match) => match[1]);
    expect(managedCodes).toHaveLength(22);
    for (const code of [...directCodes, ...managedCodes]) expect(contract).toContain(code);

    const persistence = source("src/persistence.ts");
    const policy = persistence.slice(
      persistence.indexOf("export function isRetryableSaveFailure"),
      persistence.indexOf("export type SaveFailureDisposition"),
    );
    const frontendCodes = [...policy.matchAll(/"([a-z][a-z_]*(?:\.[a-z][a-z_]*)+)"/g)]
      .map((match) => match[1])
      .filter((code) => code !== "managed.conflict");
    const producerUnion = new Set([...directCodes, ...managedCodes]);
    expect(frontendCodes.filter((code) => !producerUnion.has(code))).toEqual([]);
  });

  it("phase-B contract artifacts are complete", () => {
    const rustDir = join(process.cwd(), "src-tauri/src");
    const rustFiles = readdirSync(rustDir)
      .filter((file) => file.endsWith(".rs"))
      .map((file) => ({ file, text: readFileSync(join(rustDir, file), "utf8") }));
    expect(rustFiles.filter(({ text }) => hasStringErrorResult(text)).map(({ file }) => file)).toEqual([]);
    expect(
      rustFiles.filter(({ text }) => /map_err\(\|\w+\|\s*\w+\.to_string\(\)\)/.test(text)).map(({ file }) => file),
    ).toEqual([]);

    const commandError = source("src-tauri/src/command_error.rs");
    const parity = source("src-tauri/src/backend_command_parity.rs");
    const contract = source("docs/contracts/typed-errors.md");
    expect(commandError).toContain("PHASE_B_PRODUCER_MANIFEST");
    expect(commandError).toContain("phase_b_production_wire_matches_legacy");
    expect(parity).toContain("phase_b_command_error_manifest_is_exact_for_every_target");
    // Count and placement are pinned as ONE tuple on purpose: asserting them
    // separately lets a count-preserving swap (a Plugin failure re-mapped as
    // Backup) satisfy the count while the fingerprint silently moves.
    expect(parity).toContain("(498, 17_903_180_402_005_549_371)");
    // A mismatch must print the rows, not a bare 64-bit number nobody can act on.
    expect(parity).toContain("site_rows.join");
    expect(contract).toContain("### Absolute phase-B rule");
    expect(contract).not.toContain("### E2b phase-B pin");
    for (const family of [
      "Json", "Plugin", "Clipboard", "Platform", "GraphVerification", "Graph",
      "SyncRuntime", "Settings", "Diagnostic", "Backup",
    ]) expect(commandError).toContain(`${family} {`);
    for (const file of [
      "backup.rs", "conflict_capsule.rs", "debug.rs", "graph.rs",
      "graph_verification.rs", "platform.rs", "plugins.rs", "settings.rs",
      "storage_mode_supervisor.rs", "sync_runtime.rs",
    ]) expect(contract).toContain(`\`${file}\``);

    const proseStart = contract.indexOf("### `Prose` census");
    const proseEnd = contract.indexOf("\n##", proseStart + 1);
    const proseRows = contract.slice(proseStart, proseEnd).split("\n")
      .filter((line) => line.startsWith("|") && !line.includes("---")).slice(1);
    const allowedProse = new Map<string, Set<string>>();
    for (const row of proseRows) {
      const cells = row.split("|").slice(1, -1).map((cell) => cell.trim());
      const files = [...cells[0].matchAll(/`([^`]+\.rs)`/g)].map((match) => match[1]);
      const symbols = [...cells[1].matchAll(/`([^`]+)`/g)].map((match) => match[1]);
      for (const file of files) {
        const owned = allowedProse.get(file) ?? new Set<string>();
        for (const symbol of symbols) owned.add(symbol);
        allowedProse.set(file, owned);
      }
    }
    const misplaced: string[] = [];
    for (const { file, text } of rustFiles) {
      if (["commands.rs", "state.rs", "command_error.rs", "backend_command_parity.rs"].includes(file)) continue;
      const production = file === "sync_runtime.rs"
        ? text.slice(0, text.indexOf("#[cfg(test)]\nmod tests"))
        : withoutRustTestModules(text);
      for (const match of production.matchAll(/CommandError::prose\b/g)) {
        const symbol = enclosingRustSymbol(production, match.index);
        if (symbol === null || !allowedProse.get(file)?.has(symbol)) misplaced.push(`${file}::${symbol ?? "<none>"}`);
      }
    }
    expect(misplaced, "I-9: every phase-B Prose production site must stay in its contract-pinned symbol").toEqual([]);
  });
});
