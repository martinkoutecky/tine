// **The type-mirror consistency test** (SPEC §3.1, §7.1; P0-ts T1).
//
// `src/editor/queryIr.ts` is a HAND-WRITTEN mirror of the Rust query IR. The
// danger with a hand-written mirror is not that it disagrees loudly but that it
// agrees silently: a variant Rust added and TypeScript never learned about is,
// to `JSON.parse`, just an object, and it flows through the app untyped until
// something reads a field that is not there — or worse, until the app SAVES its
// misreading back over the author's query.
//
// So the mirror is checked, not asserted. `crates/tine-core/tests/fixtures/
// query-ir/*.json` are the golden wire bytes Rust already round-trips
// (`crates/tine-core/tests/query_ir_wire.rs`). This test reads the SAME files
// and requires their variants to be exhaustively handled by the mirror's own visitor, which throws on an
// unknown `kind` rather than skipping it.
//
// Runtime visitors check variant vocabulary; generic JSON reads below do not
// establish compile-time assignability of fixture payloads.

import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ANCHORS,
  ATTRS,
  CMP_OPS,
  DIAGNOSTIC_KINDS,
  QUANTS,
  RELS,
  UnknownIrVariantError,
  assertMirrorsIr,
  forEachFilter,
  isInvalid,
  sourceOptions,
  sourceOriginal,
} from "./queryIr";
import type {
  Bounds,
  Diagnostic,
  Filter,
  Leaf,
  Query,
  QueryResult,
  RegistrySnapshot,
  ScopedDisplaySettings,
  Source,
  ViewSettings,
} from "./queryIr";

const FIXTURE_DIR = fileURLToPath(
  new URL("../../crates/tine-core/src/query/fixtures/query-ir", import.meta.url),
);
const read = <T>(name: string): T => JSON.parse(readFileSync(join(FIXTURE_DIR, `${name}.json`), "utf8"));

// Typed reads support the tests; assertMirrorsIr performs runtime variant checks.
const query = read<Query>("query");
const queryPageAnchor = read<Query>("query_page_anchor");
const filter = read<Filter>("filter");
const leaves = read<Leaf[]>("leaf");
const sources = read<Source[]>("source");
const viewSettings = read<ViewSettings>("view_settings");
const bounds = read<Bounds>("bounds");
const registrySnapshot = read<RegistrySnapshot>("registry_snapshot");
const resultBlock = read<QueryResult>("query_result_block");
const resultPage = read<QueryResult>("query_result_page");
const diagnostics = read<Diagnostic[]>("diagnostic");
interface ScopedDisplayFixture {
  name: string;
  properties: [string, string][];
  expected: ScopedDisplaySettings;
}
const scopedDisplaySettings = read<ScopedDisplayFixture[]>("scoped_display_settings");

describe("the TypeScript mirror of the Rust query IR", () => {
  it("accepts every golden wire fixture Rust round-trips", () => {
    // Every `.json` in the shared directory must be claimed above. A fixture
    // Rust added and the mirror never read is exactly the silent gap this test
    // exists to close, so an unclaimed file is a failure, not an omission.
    const CLAIMED = new Set([
      "query", "query_page_anchor", "filter", "leaf", "source", "view_settings",
      "bounds", "registry_snapshot", "query_result_block", "query_result_page",
      "diagnostic", "scoped_display_settings",
    ]);
    const present = readdirSync(FIXTURE_DIR)
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.replace(/\.json$/, ""))
      .sort();
    const unclaimed = present.filter((name) => !CLAIMED.has(name));
    expect(
      unclaimed,
      `New golden IR fixture(s) ${unclaimed.join(", ")} that src/editor/queryIr.ts does not mirror.\n`
      + `Add the matching TypeScript shape and bind the fixture in this test — a wire\n`
      + `variant the frontend cannot name is a variant it will silently mis-handle (§3.1).`,
    ).toEqual([]);
  });

  it("preserves scoped display absence, marked emptiness, and explicit empty members", () => {
    const byName = (name: string) => scopedDisplaySettings.find((fixture) => fixture.name === name)!;
    expect(byName("missing scoped properties").expected.page_display).toBeUndefined();
    expect(byName("marked empty is present").expected.page_display).toEqual({});

    const explicit = byName("explicit empty lists and grouping clear survive").expected.page_display!;
    expect(explicit.sort).toEqual([]);
    expect(explicit.group_by).toBe("");
    expect(explicit.columns).toEqual([]);
    expect(explicit.aggregates).toEqual([]);

    const malformed = byName(
      "malformed page member and scope leave valid block state usable",
    ).expected;
    expect(malformed.page_display).toBeUndefined();
    expect(malformed.block_display?.columns).toEqual(["task", "priority"]);
    expect(malformed.unreadable_settings).toEqual([
      "tine.page-sample",
      "tine.page-match-scope",
    ]);
    expect(JSON.parse(JSON.stringify(scopedDisplaySettings))).toEqual(scopedDisplaySettings);
  });

  it("walks every variant in the corpus without meeting one it does not know", () => {
    // `assertMirrorsIr` throws `UnknownIrVariantError` on an unrecognised `kind`
    // anywhere in the tree — filter node, leaf, value, source, anchor or
    // diagnostic kind. Passing here is the mirror's claim to be complete.
    expect(() => assertMirrorsIr(query)).not.toThrow();
    expect(() => assertMirrorsIr(queryPageAnchor)).not.toThrow();
    for (const source of sources) {
      expect(() => assertMirrorsIr({ ...query, source })).not.toThrow();
    }
  });

  it("covers the WHOLE vocabulary, not merely the shapes the corpus happens to use", () => {
    // A mirror that knew three operators would also pass the walk above. Assert
    // the corpus actually exercises the vocabulary, so this test keeps its teeth.
    const nodes: Filter[] = [];
    forEachFilter(filter, (node) => nodes.push(node));
    const kinds = new Set(nodes.map((n) => n.kind));
    for (const kind of ["and", "or", "not", "leaf", "off", "raw", "true", "false"]) {
      expect(kinds, `filter variant ${kind} missing from the corpus`).toContain(kind);
    }
    const leaves = nodes.flatMap((n) => (n.kind === "leaf" ? [n.leaf] : []));
    const attrs = new Set(leaves.flatMap((l) => (l.kind === "attr" ? [l.attr] : [])));
    const ops = new Set(leaves.flatMap((l) => (l.kind === "attr" ? [l.op] : [])));
    const rels = new Set(leaves.flatMap((l) => (l.kind === "rel" ? [l.rel] : [])));
    const quants = new Set(leaves.flatMap((l) => (l.kind === "rel" ? [l.quant] : [])));
    expect([...attrs].sort()).toEqual([...ATTRS].sort());
    expect([...ops].sort()).toEqual([...CMP_OPS].sort());
    expect([...rels].sort()).toEqual([...RELS].sort());
    expect([...quants].sort()).toEqual([...QUANTS].sort());
  });

  it("REJECTS a variant it does not know, loudly", () => {
    // The negative half. A mirror that silently ignored an unknown variant would
    // pass every test above; this is the one that proves it does not.
    const alien = { kind: "quantum", items: [] } as unknown as Filter;
    expect(() => assertMirrorsIr({ ...query, filter: alien })).toThrow(UnknownIrVariantError);
    expect(() => assertMirrorsIr({ ...query, filter: alien })).toThrow(/Unknown query IR variant/);
    // Nested, not only at the root.
    expect(() => assertMirrorsIr({ ...query, filter: { kind: "not", inner: alien } }))
      .toThrow(UnknownIrVariantError);
    // And in the leaf, value, anchor, source and diagnostic positions.
    const alienLeaf = { kind: "leaf", leaf: { kind: "spooky" } } as unknown as Filter;
    expect(() => assertMirrorsIr({ ...query, filter: alienLeaf })).toThrow(UnknownIrVariantError);
    const alienValue = {
      kind: "leaf",
      leaf: { kind: "attr", attr: "content", op: "eq", value: { kind: "colour" } },
    } as unknown as Filter;
    expect(() => assertMirrorsIr({ ...query, filter: alienValue })).toThrow(UnknownIrVariantError);
    const alienAttr = {
      kind: "leaf",
      leaf: { kind: "attr", attr: "vibes", op: "eq", value: { kind: "none" } },
    } as unknown as Filter;
    expect(() => assertMirrorsIr({ ...query, filter: alienAttr })).toThrow(/Attr/);
    expect(() => assertMirrorsIr({ ...query, anchor: "sentence" as never })).toThrow(/Anchor/);
    expect(() => assertMirrorsIr({ ...query, source: { kind: "psychic" } as never }))
      .toThrow(/Source/);
    expect(() => assertMirrorsIr({
      ...query,
      diagnostics: [{ kind: "vibes" as never, message: "x" }],
    })).toThrow(/DiagnosticKind/);
  });

  it("mirrors Source's two readers exactly (§3.1: one owner for the options map)", () => {
    const og = sources.find((s) => s.kind === "og")!;
    expect(sourceOriginal(og)).toBe('(task TODO) {:title "T"}');
    expect(sourceOptions(og)).toBe('{:title "T"}');
    const builder = sources.find((s) => s.kind === "builder")!;
    // A builder query has no authored text to preserve — null, not "".
    expect(sourceOriginal(builder)).toBeNull();
    expect(sourceOptions(builder)).toBe("");
    // An absent `og_options` reads as "", never undefined: printers concatenate it.
    expect(sourceOptions({ kind: "tql", original: "task = 'TODO'" })).toBe("");
  });

  it("mirrors is_invalid: an ENABLED diagnostic invalidates, a disabled one does not", () => {
    // §3.5 — a diagnostic inside an `off` subtree carries `disabled: true` and
    // renders greyed WITHOUT zeroing the query. Getting this backwards would make
    // a disabled row silently empty every result.
    expect(isInvalid(query)).toBe(true);
    expect(isInvalid({ ...query, diagnostics: [] })).toBe(false);
    expect(isInvalid({
      ...query,
      diagnostics: [{ kind: "syntax", message: "x", disabled: true }],
    })).toBe(false);
  });

  it("mirrors the two result anchors as a discriminated union (K16)", () => {
    expect(resultBlock.anchor).toBe("block");
    expect(resultPage.anchor).toBe("page");
    // The point of the discriminant: a page result carries `pages`, never
    // `groups`, so a consumer cannot read block rows off a page-anchored answer.
    if (resultPage.anchor === "page") {
      expect(Array.isArray(resultPage.pages)).toBe(true);
      expect(resultPage.pages[0]).toMatchObject({
        path: "pages/home.md",
        properties: [["status", "active"]],
      });
      expect(resultPage.matched_total).toBe(7);
    }
    if (resultBlock.anchor === "block") {
      expect(Array.isArray(resultBlock.groups)).toBe(true);
      expect(resultBlock.matched_total).toBeUndefined();
    }
  });

  it("mirrors the enum lists against the fixtures' own spellings", () => {
    expect([...ANCHORS]).toEqual(["block", "page"]);
    expect(ANCHORS).toContain(query.anchor);
    expect(ANCHORS).toContain(queryPageAnchor.anchor);
    for (const d of query.diagnostics ?? []) expect(DIAGNOSTIC_KINDS).toContain(d.kind);
    // `raw`'s retained kind is on the wire as `diagnostic_kind`, because `kind`
    // is already the union tag. A mirror that used `kind` here would read
    // `"raw"` as a diagnostic kind and lose the payload's classification (§4.3.2).
    const raws: Filter[] = [];
    forEachFilter(filter, (n) => { if (n.kind === "raw") raws.push(n); });
    expect(raws.length).toBeGreaterThan(0);
    for (const r of raws) {
      if (r.kind !== "raw") continue;
      expect(DIAGNOSTIC_KINDS).toContain(r.diagnostic_kind);
      expect(r.text).not.toBe("");
    }
  });

  it("mirrors the standalone leaf, view, bounds and registry fixtures", () => {
    // `leaf.json` is an ARRAY: the standalone fixture pins BOTH leaf variants.
    expect(leaves.map((l) => l.kind)).toEqual(["attr", "rel"]);
    expect(typeof bounds.max_rows).toBe("number");
    expect(typeof bounds.max_bytes).toBe("number");
    expect(typeof registrySnapshot.generation).toBe("number");
    // The standalone diagnostic fixture spells EVERY kind, in order, alternating
    // the disabled flag — so the mirror's kind list is pinned against the wire
    // rather than against itself.
    expect(diagnostics.map((d) => d.kind)).toEqual([...DIAGNOSTIC_KINDS]);
    expect(diagnostics.map((d) => d.disabled)).toEqual([false, true, false, true, false, true]);
    expect(Array.isArray(registrySnapshot.rows)).toBe(true);
    // View settings are SEPARATE from the filter (Q15): nothing presentational
    // may appear inside a `Filter`, and nothing filtering inside `ViewSettings`.
    expect(Object.keys(viewSettings).every((k) =>
      ["view", "sort", "group_by", "columns", "aggregates", "sample"].includes(k),
    )).toBe(true);
  });
});
it("validates finite statistics cells and every marker in the shared wire fixtures", async () => {
  const { isQueryStatistics } = await import("./queryIr");
  for (const result of [resultBlock, resultPage]) {
    expect(isQueryStatistics(result.statistics)).toBe(true);
    expect(isQueryStatistics(JSON.parse(JSON.stringify(result.statistics)))).toBe(true);
    for (const value of [null, "3.125", Infinity, NaN]) {
      const bad = structuredClone(result.statistics!);
      Object.assign(bad.overall[0], { value });
      expect(isQueryStatistics(bad)).toBe(false);
    }
    const bad = structuredClone(result.statistics!);
    Object.assign(bad.overall[2], { value: null });
    expect(isQueryStatistics(bad)).toBe(false);
  }
});
