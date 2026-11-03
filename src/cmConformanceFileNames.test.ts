// Conceptual model conformance, step 2 batch 3 (census Q2, "what is a file's
// page name"): the frontend's own computers of that answer, run against the
// store's answer on every fixture graph.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_tests.rs`,
// `file_name_answerers`) writes one case per fixture with a page header or an
// Org file link to `crates/tine-store/tests/fixtures/cm-conformance/
// frontend-names/` (and fails when a committed case is stale): per ordinary
// page its format, the `pre_block` its `PageDto` carries, the store's title
// (`page_title_from_preamble`) and the store's name for the file (the
// inventory); per Org block with a `[[file:…]]` link its raw text and the
// linked file's name.
//
// Checked here:
// - `document/host/wiring.ts settleTitleIdentity`, which after a save renames
//   the loaded page to the `title::` it reads from `dto.pre_block` with
//   `pagePropertyEntries` (TS) instead of receiving the backend's name. The
//   function is module-private and drives the host, so its title rule is
//   restated here in one line (`settledTitle`, wiring.ts:563-564).
// - `render/pageRefs.ts pageRefsInText` (rename candidates, graph rewrite,
//   swipe actions; `editor/nearestLink.ts` calls the same `linkTargetName` for
//   follow-link) with the graph's file-name format installed as graph.ts does:
//   the name an Org file link names, which must be the linked file's page name.
//
// Differences are compared with the `frontendNames` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { pagePropertyEntries } from "./editor/properties";
import { pageRefsInText, setFileNameFormat } from "./render/pageRefs";

interface Case {
  /** The graph's `:file/name-format`, as GraphMeta `file_name_format` carries it. */
  fileNameFormat: "legacy" | "triple-lowbar";
  files: { path: string; format: "md" | "org"; preBlock: string | null; title: string | null; name: string | null }[];
  links: { path: string; raw: string; target: string; name: string }[];
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");

function cases(): [string, Case][] {
  const dir = path.join(FIXTURES, "frontend-names");
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => [f.slice(0, -".json".length), JSON.parse(readFileSync(path.join(dir, f), "utf8")) as Case]);
}

/** Sorted keys, so the file matches what serde_json (BTreeMap) writes. */
function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

/** wiring.ts:563-564 `settleTitleIdentity`'s title read, restated. */
const settledTitle = (preBlock: string | null, format: "md" | "org") =>
  pagePropertyEntries(preBlock, format === "org" ? "org" : "md").find((entry) => entry.key.toLowerCase() === "title")?.value;

function runCase(c: Case): string[] {
  const diffs: string[] = [];
  // graph.ts applyConfigDerivedState: setFileNameFormat(meta.file_name_format).
  setFileNameFormat(c.fileNameFormat);
  for (const f of c.files) {
    const ts = settledTitle(f.preBlock, f.format) || null;
    if (ts !== f.title) {
      diffs.push(`frontend title rule: settleTitleIdentity reads ${JSON.stringify(ts)} in ${f.path}, the store's title is ${JSON.stringify(f.title)}`);
    }
    if (ts && ts !== f.name) {
      diffs.push(`frontend title rule: settleTitleIdentity would rename ${f.path} to ${JSON.stringify(ts)}, the store names it ${JSON.stringify(f.name)}`);
    }
  }
  for (const l of c.links) {
    const names = pageRefsInText(l.raw, "org");
    if (!names.includes(l.name)) {
      diffs.push(`frontend file link: pageRefsInText reads ${JSON.stringify(names)} in ${l.path} (${JSON.stringify(l.raw)}), ${l.target} is named ${JSON.stringify(l.name)}`);
    }
  }
  return diffs.sort();
}

beforeAll(() => initParser());

describe("conceptual model conformance: the frontend's file-name computers", () => {
  it("differ from the store's answer exactly as expected-differences.json records", () => {
    const now: Record<string, string[]> = {};
    for (const [name, c] of cases()) {
      const diffs = runCase(c);
      if (diffs.length) now[name] = diffs;
    }
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontendNames = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontendNames as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the frontend's file-name differences changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceFileNames.test.ts",
    ).toEqual(was);
  });
});
