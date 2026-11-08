// Conceptual model conformance, step 2 batch 10 (namespace parents, census Q5
// item 3, divergence S8): the frontend's namespace views, run against the
// conceptual model.
//
// The model (rule 1, SPEC §3.2, CM3-3) says a name `n/y` makes `n` a page. The
// Rust harness writes, per fixture graph, the store's `page_inventory` entries
// (wire shape) and the model's page set (`pages`). This test installs that
// inventory in `pageIndex.ts` exactly as the app does (a mocked
// `page_inventory` IPC) and runs, for real, every view that nests names by `/`:
// - the namespace tree (`buildNamespaceTree` over `allPageNames()`: the left
//   sidebar tree and the Namespaces view);
// - the Namespaces view's per-row count (`indexNamespaces`);
// - the `{{namespace X}}` macro's tree (`namespaceNode`);
// - the "Hierarchy" section of every page (`namespaceHierarchyRows`).
// Each view's answer is compared with the model's: a page's parent is the
// model's graph-aware `treeParent`; nodes, children and the global subset are
// exported directly by r11's `tree` and `namespaceTree`. The test reconstructs
// neither a page universe from inventory names nor parent edges from prefixes.
//
// Differences are compared with the `frontendNamespaces` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PageInventoryEntry } from "./types";

const backendMock = vi.hoisted(() => ({ pageInventory: vi.fn(), graphBindingGeneration: vi.fn(() => 1) }));
vi.mock("./backend", () => ({ backend: () => backendMock }));

interface ModelPage { kind: "page" | "journal"; key: string; name: string; parent: string | null }
interface Case {
  journalTitleFormat: string | null;
  journalFileFormat: string | null;
  inventory: PageInventoryEntry[];
  pages: ModelPage[];
  treeParent: { page: [string, string]; parent: [string, string] | null }[];
  tree: { page: [string, string]; children: [string, string][] }[];
  namespaceTree: { page: [string, string]; children: [string, string][] }[];
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");

function cases(): [string, Case][] {
  const dir = path.join(FIXTURES, "frontend");
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

const list = (keys: Iterable<string>) => `[${[...keys].sort().join(", ")}]`;

async function runCase(c: Case): Promise<string[]> {
  backendMock.pageInventory.mockReset().mockResolvedValue({ rev: "1", entries: c.inventory });
  await (await import("./render/parse")).initParser();
  (await import("./journal")).setJournalFormats(c.journalTitleFormat, c.journalFileFormat);
  const index = await import("./pageIndex");
  // Each graph gets fresh inventory state without reinitializing the module tree.
  index.resetPageIndex();
  index.installPageIndex();
  await index.refreshPageIndex();
  await vi.waitFor(() => expect(index.allPages()).toBeDefined());
  const ns = await import("./components/Namespace");
  const nsList = await import("./namespacesList");
  const { pageIdentityKey } = await import("./pageIdentity");
  const diffs: string[] = [];

  // The model's namespace structure over its page keys (pages of either kind:
  // `allPageNames` lists journals too).
  const model = new Map<string, string>(); // key -> name
  for (const page of c.pages) {
    if (model.has(page.key)) continue;
    model.set(page.key, page.name);
  }
  const parentOf = new Map(c.treeParent.map((row) => [row.page[1], row.parent?.[1] ?? null]));
  const childrenOf = new Map(c.tree.map((row) => [row.page[1], row.children.map((p) => p[1])]));
  const modelParent = (key: string): string | null => parentOf.get(key) ?? null;
  // The model pages below `key`: those whose parent chain reaches it.
  const below = (key: string) => {
    const found = new Set<string>();
    const pending = [...(childrenOf.get(key) ?? [])];
    while (pending.length) {
      const child = pending.pop()!;
      if (found.has(child)) continue;
      found.add(child);
      pending.push(...(childrenOf.get(child) ?? []));
    }
    return [...found];
  };
  const modelNodes = new Map(c.namespaceTree.map((row) => [row.page[1], modelParent(row.page[1])]));

  // The tree the sidebar and the Namespaces view show.
  const names = index.allPageNames();
  const tree = ns.buildNamespaceTree(names);
  const shown = new Map<string, string | null>();
  const pending = tree.map((node) => ({ node, parent: null as string | null }));
  while (pending.length) {
    const { node, parent } = pending.pop()!;
    const key = pageIdentityKey(node.full);
    shown.set(key, parent);
    for (const child of node.children) pending.push({ node: child, parent: key });
  }
  for (const [key, parent] of modelNodes) {
    if (!shown.has(key)) diffs.push(`frontend namespace tree: model has node "${key}" (under ${parent ?? "the top"}), Tine does not`);
    else if (shown.get(key) !== parent) {
      diffs.push(`frontend namespace tree: "${key}" sits under ${parent ?? "the top"} in the model, under ${shown.get(key) ?? "the top"} in Tine`);
    }
  }
  for (const [key] of shown) {
    if (!modelNodes.has(key)) diffs.push(`frontend namespace tree: Tine has node "${key}", the model does not`);
  }
  const listed = new Set(names.map((name) => pageIdentityKey(name)));
  for (const [key] of shown) {
    if (!listed.has(key)) diffs.push(`frontend namespace tree: node "${key}" is not a name the inventory lists`);
  }

  // The Namespaces view's counts.
  for (const entry of nsList.indexNamespaces(tree)) {
    const key = pageIdentityKey(entry.node.full);
    if (!model.has(key)) continue;
    const want = below(key).length;
    if (entry.count !== want) diffs.push(`frontend namespaces count "${key}": model ${want}, Tine ${entry.count}`);
  }

  // `{{namespace X}}` and the Hierarchy section, for every model page.
  for (const [key, name] of model) {
    const want = below(key);
    const node = ns.namespaceNode(
      names.filter((n) => { const k = pageIdentityKey(n); return k === key || k.startsWith(`${key}/`); }),
      name,
    );
    const macro = new Set<string>();
    for (const row of node ? ns.namespaceRows(node.children) : []) macro.add(pageIdentityKey(row.node.full));
    if (list(macro) !== list(want)) diffs.push(`frontend {{namespace ${key}}}: model ${list(want)}, Tine ${list(macro)}`);
    const rows = ns.namespaceHierarchyRows(names, name).map((segs) => pageIdentityKey(segs.join("/")));
    const wantRows = want.length ? want : modelParent(key) !== null ? [modelParent(key)!] : [];
    if (list(rows) !== list(wantRows)) diffs.push(`frontend hierarchy "${key}": model ${list(wantRows)}, Tine ${list(rows)}`);
  }
  return diffs.sort();
}

beforeEach(() => {
  vi.resetModules();
  backendMock.graphBindingGeneration.mockReset().mockReturnValue(1);
});

describe("conceptual model conformance: the frontend's namespace views", () => {
  it("differ from the model exactly as expected-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    for (const [name, c] of cases()) {
      const diffs = await runCase(c);
      if (diffs.length) now[name] = diffs;
    }
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontendNamespaces = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontendNamespaces as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the namespace views' differences changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceNamespaces.test.ts",
    ).toEqual(was);
  }, 60_000);
});
