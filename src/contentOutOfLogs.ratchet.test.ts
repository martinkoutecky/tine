import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parserDiagnostic } from "./devtools/lsdoc-diff/diagnostic";

const source = (path: string) => readFileSync(join(process.cwd(), path), "utf8");
const SRC = fileURLToPath(new URL(".", import.meta.url));

interface ConsoleSite {
  file: string;
  line: number;
  method: "log" | "warn" | "error" | "debug";
}

// Every console site carries one of four buckets. They are the same four the
// Rust print census uses (`crates/tine-core/tests/content_out_of_logs.rs`), so
// one classification answers "is this line safe?" on both sides:
//
//   a — content-free or fixed-shape payload, and gated behind a debug opt-in.
//   b — a directed investigation channel behind its OWN named opt-in; may
//       carry detail, because the user asked for it.
//   c — always-on, with a variable that CAN carry user content (a page name,
//       a graph path, block text, or error prose from an operation over any
//       of those). MUST BE ZERO; the assertion below enforces it.
//   d — always-on, payload provably content-free.
//
// Nothing on the frontend is gated: the WebView inspector ships in release
// builds (`src-tauri/Cargo.toml`, feature `devtools`), so every site here is
// either (c) or (d) and (c) is empty. `failureShape()` is what moved ten rows
// from (c) to (d): it keeps a failure's type, size and identity and drops its
// message.
type ConsoleBucket = "a" | "b" | "c" | "d";
const CONSOLE_ALLOWLIST_SIZE = 21;
const CONSOLE_ALLOWLIST: readonly (ConsoleSite & { bucket: ConsoleBucket; class: string; why: string })[] = [
  { file: "App.tsx", line: 1071, method: "warn", bucket: "d", class: "local-error", why: "SafeBack listener registration failed; a Tauri plugin-setup error names no graph object" },
  { file: "capture.tsx", line: 173, method: "log", bucket: "d", class: "numeric-shape", why: "capture-window sizing measurements contain only numbers" },
  { file: "capture.tsx", line: 600, method: "error", bucket: "d", class: "local-error", why: "wasm module init failure; the parser is handed no document at bootstrap" },
  { file: "components/Block.tsx", line: 1533, method: "warn", bucket: "d", class: "scrubbed-error", why: "failureShape() — the facet query carries the property prefix being typed" },
  { file: "logbook.ts", line: 43, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — the marker transition runs over the block's own text" },
  { file: "main.tsx", line: 62, method: "error", bucket: "d", class: "local-error", why: "window reveal failure is a native window-manager error, not a graph operation" },
  { file: "main.tsx", line: 70, method: "error", bucket: "d", class: "local-error", why: "wasm module init failure; the parser is handed no document at bootstrap" },
  { file: "pdfRenderCoordinator.ts", line: 343, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — pdf.js render rejections name the document they failed on" },
  { file: "persistence.ts", line: 999, method: "warn", bucket: "d", class: "numeric-shape", why: "save refusal carries only a count" },
  { file: "persistence.ts", line: 1004, method: "error", bucket: "d", class: "numeric-shape", why: "save refusal carries only a count" },
  { file: "persistence.ts", line: 1152, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — a managed conflict capture error is prose about the saved page" },
  { file: "print.ts", line: 98, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — a KaTeX/highlight rejection quotes the source it refused" },
  { file: "print.ts", line: 113, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — pagePrintHtml errors name the page" },
  { file: "print.ts", line: 157, method: "error", bucket: "d", class: "local-error", why: "iframe print failure is a DOM/print-subsystem error over no page identity" },
  { file: "render/parse.ts", line: 51, method: "warn", bucket: "d", class: "build-token", why: "compares two public parser build tags" },
  { file: "sheet/formulaEval.ts", line: 193, method: "warn", bucket: "d", class: "internal-id-count", why: "performance warning carries an internal owner id and numeric count" },
  { file: "store.ts", line: 6969, method: "warn", bucket: "d", class: "scrubbed-error", why: "failureShape() — replay-evidence retirement errors carry the private store path" },
  { file: "ui.ts", line: 491, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule persistence errors carry the conflicted page and path" },
  { file: "ui.ts", line: 516, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule refresh errors carry the conflicted page and path" },
  { file: "ui.ts", line: 553, method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule retirement errors carry the conflicted page and path" },
  { file: "update.ts", line: 148, method: "error", bucket: "d", class: "scrubbed-error", why: "safeUpdaterErrorChain permits only classified updater stages and causes" },
];

function sourceFiles(dir: string, files: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      sourceFiles(full, files);
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry) && !/^testSetup\./.test(entry)) {
      files.push(full);
    }
  }
  return files;
}

function closingParen(text: string, open: number): number {
  let depth = 1;
  let quote: "\"" | "'" | "`" | null = null;
  let escaped = false;
  for (let index = open + 1; index < text.length; index += 1) {
    const char = text[index];
    if (quote) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === quote) quote = null;
      continue;
    }
    if (char === "\"" || char === "'" || char === "`") quote = char;
    else if (char === "(") depth += 1;
    else if (char === ")" && --depth === 0) return index;
  }
  throw new Error("unterminated console call");
}

function isFixedLiteral(argumentsText: string): boolean {
  return /^\s*(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\$]|\\.|\$(?!\{))*`)\s*,?\s*$/s.test(argumentsText);
}

function variableConsoleSites(): ConsoleSite[] {
  const sites: ConsoleSite[] = [];
  for (const file of sourceFiles(SRC)) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/\bconsole\.(log|warn|error|debug)\s*\(/g)) {
      const open = match.index! + match[0].lastIndexOf("(");
      const end = closingParen(text, open);
      if (isFixedLiteral(text.slice(open + 1, end))) continue;
      sites.push({
        file: relative(SRC, file).replaceAll("\\", "/"),
        line: text.slice(0, match.index).split("\n").length,
        method: match[1] as ConsoleSite["method"],
      });
    }
  }
  return sites.sort((left, right) => left.file.localeCompare(right.file) || left.line - right.line);
}

describe("I-5 content-out-of-logs ratchet", () => {
  it("equals the reviewed production console census", () => {
    expect(CONSOLE_ALLOWLIST).toHaveLength(CONSOLE_ALLOWLIST_SIZE);
    for (const entry of CONSOLE_ALLOWLIST) {
      expect(entry.class, `${entry.file}:${entry.line} needs a class`).not.toBe("");
      expect(entry.why, `${entry.file}:${entry.line} needs a reason`).not.toBe("");
    }
    expect(
      CONSOLE_ALLOWLIST.filter((entry) => entry.bucket === "c").map((entry) => `${entry.file}:${entry.line}`),
      "I-5: class (c) is always-on plus a variable that can carry user content, and it is ZERO here. "
        + "Do not classify a site into (c); fix it — pass the value through failureShape() (exemplar: "
        + "src/failureShape.ts, used at ui.ts) or log a count (exemplar: persistence.ts logs `{ count }`)",
    ).toEqual([]);
    expect(
      variableConsoleSites(),
      "I-5: the variable-bearing console census changed. Log a count or a fixed string, never user content "
        + "(exemplar: persistence.ts logs `{ count }`); if a new site is legitimately content-free, add it to "
        + "CONSOLE_ALLOWLIST with its class and reason and bump CONSOLE_ALLOWLIST_SIZE",
    ).toEqual(
      CONSOLE_ALLOWLIST.map(({ file, line, method }) => ({ file, line, method })),
    );
  });

  it("pins the diagnostics contract to both allowlist sizes and gates", () => {
    const contract = source("docs/contracts/diagnostics.md");
    const rustRatchet = source("crates/tine-core/tests/content_out_of_logs.rs");
    expect(contract).toContain("74 Rust production print sites");
    expect(contract).toContain("21 variable-bearing frontend console sites");
    expect(contract).toContain("debug_enabled()");
    expect(contract).toContain("runtime_debug_diagnostics_enabled()");
    expect(rustRatchet).toContain("const RUST_PRINT_SITE_COUNT: usize = 74;");
  });

  it("makes parser failures fixed-shape before they cross the lsdoc-diff worker boundary", () => {
    const worker = source("src/devtools/lsdoc-diff/worker.ts");
    const client = source("src/devtools/lsdoc-diff/mldoc-client.ts");
    const orchestrator = source("src/devtools/lsdoc-diff/orchestrator.ts");

    for (const text of [worker, client, orchestrator]) {
      expect(text).not.toMatch(/detail:\s*String\s*\(/);
      expect(text).not.toMatch(/detail:\s*m\.detail\b/);
    }
    expect(worker).not.toMatch(/loadError\s*=\s*`[^`]*\$\{/);
    expect(worker).toContain("diagnostic:");
    expect(client).toContain("diagnostic:");
    expect(orchestrator).toContain("diagnostic:");
  });

  it("represents parser input only by offset, byte length, and an opaque hash", () => {
    const first = parserDiagnostic("private parser input");
    const second = parserDiagnostic("different parser input", 7);
    expect(Object.keys(first).sort()).toEqual(["inputBytes", "inputHash", "offset"]);
    expect(first.offset).toBeNull();
    expect(second.offset).toBe(7);
    expect(first.inputBytes).toBe(new TextEncoder().encode("private parser input").length);
    expect(first.inputHash).toMatch(/^[0-9a-f]{16}$/);
    expect(second.inputHash).not.toBe(first.inputHash);
  });
});
