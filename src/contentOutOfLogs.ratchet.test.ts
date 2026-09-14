import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parserDiagnostic } from "./devtools/lsdoc-diff/diagnostic";

const source = (path: string) => readFileSync(join(process.cwd(), path), "utf8");
const SRC = fileURLToPath(new URL(".", import.meta.url));

interface ConsoleSite {
  file: string;
  /// The site's IDENTITY: a digest of the call's method and its normalized
  /// argument text. Deliberately not the line number -- see the note on
  /// CONSOLE_ALLOWLIST.
  anchor: string;
  method: "log" | "warn" | "error" | "debug";
}

/// Where the site currently sits, for the failure message only. Never asserted.
interface ConsoleSiteLocation extends ConsoleSite {
  line: number;
  text: string;
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

// Rows are keyed by CONTENT, not by line number, and that is the whole point of
// the `anchor` column.
//
// This census used to pin `{ file, line }`. Every such row is a claim about a
// line number in a file the census does not otherwise care about, so inserting
// a line anywhere above one of them turned this suite red for a lane that never
// touched logging: five of six candidate models in the September model
// comparison reddened `npm test` on files they had not edited, and each one
// then had to decide whether a safety ratchet it did not understand was
// reporting a real finding. An anchor moves with its call.
//
// It also catches something the line pin actively hid. Two `print.ts` rows had
// their class and reason SWAPPED -- the row at the iframe site described the
// pagePrintHtml site and vice versa -- and the suite stayed green for as long
// as both sites were `error` and both line numbers existed. The classification
// is the safety content here; binding it to a line number rather than to the
// call meant the census could be green and wrong at the same time.
const CONSOLE_ALLOWLIST: readonly (ConsoleSite & { bucket: ConsoleBucket; class: string; why: string })[] = [
  { file: "App.tsx", anchor: "bd49af363722", method: "warn", bucket: "d", class: "local-error", why: "SafeBack listener registration failed; a Tauri plugin-setup error names no graph object" },
  { file: "capture.tsx", anchor: "3de353ced60a", method: "error", bucket: "d", class: "local-error", why: "wasm module init failure; the parser is handed no document at bootstrap" },
  { file: "capture.tsx", anchor: "c302419aed10", method: "log", bucket: "d", class: "numeric-shape", why: "capture-window sizing measurements contain only numbers" },
  { file: "components/Block.tsx", anchor: "ca982a4fd092", method: "warn", bucket: "d", class: "scrubbed-error", why: "failureShape() — the facet query carries the property prefix being typed" },
  { file: "logbook.ts", anchor: "f27172cceded", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — the marker transition runs over the block's own text" },
  { file: "main.tsx", anchor: "3de353ced60a", method: "error", bucket: "d", class: "local-error", why: "wasm module init failure; the parser is handed no document at bootstrap" },
  { file: "main.tsx", anchor: "e4a2943c031b", method: "error", bucket: "d", class: "local-error", why: "window reveal failure is a native window-manager error, not a graph operation" },
  { file: "pdfRenderCoordinator.ts", anchor: "0ad719767700", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — pdf.js render rejections name the document they failed on" },
  { file: "persistence.ts", anchor: "56ce51bc820d", method: "warn", bucket: "d", class: "numeric-shape", why: "save refusal carries only a count" },
  { file: "persistence.ts", anchor: "a6c2fe5eb40f", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — a managed conflict capture error is prose about the saved page" },
  { file: "persistence.ts", anchor: "d000164d69a2", method: "error", bucket: "d", class: "numeric-shape", why: "save refusal carries only a count" },
  { file: "print.ts", anchor: "2403b56e48d3", method: "error", bucket: "d", class: "local-error", why: "iframe print failure is a DOM/print-subsystem error over no page identity" },
  { file: "print.ts", anchor: "4411f8c9e188", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — a KaTeX/highlight rejection quotes the source it refused" },
  { file: "print.ts", anchor: "99eb03faa4fe", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — pagePrintHtml errors name the page" },
  { file: "render/parse.ts", anchor: "1e8f76713ce3", method: "warn", bucket: "d", class: "build-token", why: "compares two public parser build tags" },
  { file: "sheet/formulaEval.ts", anchor: "8259b2f56d25", method: "warn", bucket: "d", class: "internal-id-count", why: "performance warning carries an internal owner id and numeric count" },
  { file: "store.ts", anchor: "1800fecf7cc1", method: "warn", bucket: "d", class: "scrubbed-error", why: "failureShape() — replay-evidence retirement errors carry the private store path" },
  { file: "ui.ts", anchor: "22fb47f1f860", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule persistence errors carry the conflicted page and path" },
  { file: "ui.ts", anchor: "350928478727", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule retirement errors carry the conflicted page and path" },
  { file: "ui.ts", anchor: "ebb833424c2f", method: "error", bucket: "d", class: "scrubbed-error", why: "failureShape() — capsule refresh errors carry the conflicted page and path" },
  { file: "update.ts", anchor: "b7b0521bc51e", method: "error", bucket: "d", class: "scrubbed-error", why: "safeUpdaterErrorChain permits only classified updater stages and causes" },
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

/// The site's identity. Whitespace is collapsed so that reformatting a call
/// across lines does not move it; anything else about the arguments -- the
/// literal, the variable, whether it passes through failureShape() -- changes
/// the anchor, which is correct: a different payload is a different site and
/// must be reclassified.
function anchorOf(method: string, argumentsText: string): string {
  const normalized = argumentsText.replace(/\s+/g, " ").trim();
  return createHash("sha256").update(`${method}(${normalized})`).digest("hex").slice(0, 12);
}

function variableConsoleSites(): ConsoleSiteLocation[] {
  const sites: ConsoleSiteLocation[] = [];
  for (const file of sourceFiles(SRC)) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/\bconsole\.(log|warn|error|debug)\s*\(/g)) {
      const open = match.index! + match[0].lastIndexOf("(");
      const end = closingParen(text, open);
      const argumentsText = text.slice(open + 1, end);
      if (isFixedLiteral(argumentsText)) continue;
      const method = match[1] as ConsoleSite["method"];
      sites.push({
        file: relative(SRC, file).replaceAll("\\", "/"),
        anchor: anchorOf(method, argumentsText),
        method,
        line: text.slice(0, match.index).split("\n").length,
        text: argumentsText.replace(/\s+/g, " ").trim().slice(0, 120),
      });
    }
  }
  return sites.sort((left, right) => left.file.localeCompare(right.file) || left.anchor.localeCompare(right.anchor));
}

/// A site's identity, in the only two forms the assertions use.
const identity = (site: ConsoleSite) => `${site.file} ${site.method} ${site.anchor}`;
const located = (site: ConsoleSiteLocation) => `${site.file}:${site.line} ${site.method} ${site.anchor} — ${site.text}`;

describe("I-5 content-out-of-logs ratchet", () => {
  it("equals the reviewed production console census", () => {
    expect(CONSOLE_ALLOWLIST).toHaveLength(CONSOLE_ALLOWLIST_SIZE);
    for (const entry of CONSOLE_ALLOWLIST) {
      expect(entry.class, `${identity(entry)} needs a class`).not.toBe("");
      expect(entry.why, `${identity(entry)} needs a reason`).not.toBe("");
    }
    // An anchor must name one call. Two identical calls in one file would let a
    // row describe either of them, which is the ambiguity the line pin had.
    expect(new Set(CONSOLE_ALLOWLIST.map(identity)).size, "duplicate anchors in the allowlist").toBe(
      CONSOLE_ALLOWLIST.length,
    );
    const sites = variableConsoleSites();
    expect(new Set(sites.map(identity)).size, "two identical console calls in one file: give one a distinct message").toBe(
      sites.length,
    );
    expect(
      CONSOLE_ALLOWLIST.filter((entry) => entry.bucket === "c").map(identity),
      "I-5: class (c) is always-on plus a variable that can carry user content, and it is ZERO here. "
        + "Do not classify a site into (c); fix it — pass the value through failureShape() (exemplar: "
        + "src/failureShape.ts, used at ui.ts) or log a count (exemplar: persistence.ts logs `{ count }`)",
    ).toEqual([]);

    const allowed = new Set(CONSOLE_ALLOWLIST.map(identity));
    expect(
      sites.filter((site) => !allowed.has(identity(site))).map(located),
      "I-5: a variable-bearing console site is not in the reviewed census. Log a count or a fixed string, "
        + "never user content (exemplar: persistence.ts logs `{ count }`); if the payload is legitimately "
        + "content-free, add it to CONSOLE_ALLOWLIST with its anchor, class and reason and bump "
        + "CONSOLE_ALLOWLIST_SIZE. Anchors are content digests, so this does NOT fire merely because lines moved",
    ).toEqual([]);
    const present = new Set(sites.map(identity));
    expect(
      CONSOLE_ALLOWLIST.filter((entry) => !present.has(identity(entry))).map(identity),
      "I-5: a censused console site no longer exists with that payload. If you changed what it logs, "
        + "reclassify it and update its anchor; if you deleted it, drop the row and lower CONSOLE_ALLOWLIST_SIZE",
    ).toEqual([]);
  });

  it("pins the diagnostics contract to both allowlist sizes and gates", () => {
    const contract = source("docs/contracts/diagnostics.md");
    const rustRatchet = source("crates/tine-core/tests/content_out_of_logs.rs");
    expect(contract).toContain("75 Rust production print sites");
    expect(contract).toContain("21 variable-bearing frontend console sites");
    expect(contract).toContain("debug_enabled()");
    expect(contract).toContain("runtime_debug_diagnostics_enabled()");
    expect(rustRatchet).toContain("const RUST_PRINT_SITE_COUNT: usize = 75;");
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
