// Conceptual model conformance, step 2 batch 7 (census Q1 "the page-name key"
// and Q7 "the alias rule"): the frontend's own computers of those answers, run
// against the store's answers.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_tests.rs`) writes:
// - `frontend-keys/corpus.json` (`key_answerers`): tricky page names and each
//   one's `page_key` (non-ASCII case, NFD, edge whitespace, boundary slashes);
// - `frontend-aliases/headers.json` (`alias_header_cases`): per page-header
//   shape, the `PageDto` the frontend loads (pre-block, first root) and the
//   store's declared aliases (`document_alias_spellings`).
//
// Checked here:
// - `pageIdentityKey` (wasm build of `page_key`) and `seen/displayed.ts keyOf`;
// - folds inline in production that are declared different questions,
//   restated in one line each with their site (`guide.ts key`);
// - the real `orderAcItems` (`[[` completion's Enter policy): typing a spelling
//   of an existing page must make that page the default, not "Create";
// - the page header's alias list: `render/block.ts aliasNamesOf` over the
//   properties Page.tsx reads (`pageHeaderProperties`: the pre-block, then a
//   properties-only first root, restated from `pagePropertyParts`) and over
//   what RightSidebar.tsx reads (`readPageProperties`: first line per key).
//
// Differences are compared with the `frontendKeys` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { pageIdentityKey } from "./pageIdentity";
import { keyOf } from "./seen/displayed";
import { orderAcItems } from "./editor/autocomplete";
import { aliasNamesOf } from "./render/block";
import { isPropertiesOnly, pagePropertyEntries } from "./editor/properties";

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");

interface Named { name: string; key: string }
interface Header { path: string; format: "md" | "org"; preBlock: string | null; firstRoot: string | null; declared: string[] }

const read = <T>(rel: string): T => JSON.parse(readFileSync(path.join(FIXTURES, rel), "utf8")) as T;

/** Sorted keys, so the file matches what serde_json (BTreeMap) writes. */
function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

const show = (s: string) => JSON.stringify(s);

/** Production folds restated, each one line at its site. Only declared
 * different questions remain (census Q1): `pageIndex.ts` allPageNames,
 * `autocomplete.ts` exact match and `seen/displayed.ts` call
 * `pageIdentityKey`; `autocomplete.ts canonicalName` is a sort key only. */
const FOLDS: [string, (s: string) => string][] = [
  ["guide.ts key (declared: Guide titles are a separate namespace)", (s) => s.trim().toLowerCase()],
];

function corpusDiffs(corpus: Named[]): string[] {
  const diffs: string[] = [];
  for (const { name, key } of corpus) {
    if (pageIdentityKey(name) !== key) diffs.push(`pageIdentityKey(${show(name)}) = ${show(pageIdentityKey(name))}, page_key ${show(key)}`);
    if (keyOf(name) !== key) diffs.push(`seen keyOf(${show(name)}) = ${show(keyOf(name))}, page_key ${show(key)}`);
  }
  for (const [site, fold] of FOLDS) {
    for (let i = 0; i < corpus.length; i++) {
      for (let j = i + 1; j < corpus.length; j++) {
        const byKey = corpus[i].key === corpus[j].key;
        const byFold = fold(corpus[i].name) === fold(corpus[j].name);
        if (byKey !== byFold) {
          diffs.push(`${site}: ${show(corpus[i].name)} / ${show(corpus[j].name)}: page_key ${byKey ? "same" : "different"}, fold ${byFold ? "same" : "different"}`);
        }
      }
    }
  }
  // `[[` completion: the backend's row for an existing page, the user typed
  // another spelling of the same page.
  const pages = ["Foo", "Über", "Straße", "ΟΔΟΣ", "İstanbul"];
  for (const page of pages) {
    const pageKey = corpus.find((c) => c.name === page)?.key;
    for (const typed of corpus.filter((c) => c.key === pageKey && c.name !== page)) {
      const order = orderAcItems([{ name: page, item: "page" }], { name: typed.name, item: "create" }, { query: typed.name, policy: "typed" });
      if (order[0] !== "page") diffs.push(`orderAcItems: typing ${show(typed.name)} with page ${show(page)} existing: ${order[0]} is the default, not the page`);
    }
  }
  return diffs.sort();
}

/** Page.tsx `headerProperties` = `pageHeaderProperties(page)`; for a loaded
 *  page that is `pagePropertyParts` (document/edits/properties.ts:96-104):
 *  Markdown's properties-only first root joins the pre-block. */
function pageHeaderLines(h: Header): [string, string][] {
  const root = h.format === "md" && h.firstRoot !== null && isPropertiesOnly(h.firstRoot) ? h.firstRoot : null;
  const parts = h.preBlock || root === null ? [h.preBlock ?? "", ...(root === null ? [] : [root])] : [root];
  return parts.flatMap((text) => pagePropertyEntries(text, h.format).map((e): [string, string] => [e.key, e.value]));
}

/** RightSidebar.tsx `readPageProperties`: the same lines, first per key. */
function sidebarLines(h: Header): [string, string][] {
  const seen = new Set<string>();
  return pageHeaderLines(h).filter(([key]) => !seen.has(key.toLowerCase()) && !!seen.add(key.toLowerCase()));
}

function headerDiffs(headers: Header[]): string[] {
  const diffs: string[] = [];
  const keys = (names: string[]) => [...new Set(names.map(pageIdentityKey))].sort();
  for (const h of headers) {
    const store = keys(h.declared);
    for (const [surface, lines] of [["Page.tsx header", pageHeaderLines(h)], ["RightSidebar.tsx header", sidebarLines(h)]] as const) {
      const shown = keys(aliasNamesOf(lines, h.format));
      if (JSON.stringify(shown) !== JSON.stringify(store)) {
        diffs.push(`${surface}: ${h.path} shows aliases ${JSON.stringify(shown)}, the store declares ${JSON.stringify(store)}`);
      }
    }
  }
  return diffs.sort();
}

beforeAll(() => initParser());

describe("conceptual model conformance: the frontend's key and alias computers", () => {
  it("differ from the store's answers exactly as expected-differences.json records", () => {
    const now: Record<string, string[]> = {};
    const corpus = corpusDiffs(read<Named[]>("frontend-keys/corpus.json"));
    if (corpus.length) now.corpus = corpus;
    const headers = headerDiffs(read<Header[]>("frontend-aliases/headers.json"));
    if (headers.length) now.headers = headers;
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontendKeys = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontendKeys as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the frontend's key/alias differences changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceKeys.test.ts",
    ).toEqual(was);
  });
});
