import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { initParser } from "./render/parse";
import { BrowserGraph, __tineReinstantiate } from "./render/wasm/lsdoc_wasm";

// The browser core (D-20261122-WEX) answers as the desktop app's store: the
// same graph gives the rows src-tauri's discovery test pins from this fixture.
it("the browser core's page inventory is the desktop app's", async () => {
  await initParser();
  const fixture = JSON.parse(readFileSync("tests/fixtures/browser-core-inventory.json", "utf8"));
  const graph = open(fixture.files);
  const wire = JSON.parse(graph.page_inventory_json());
  graph.free();
  expect(wire.entries).toEqual(fixture.rows);
  expect(wire.unreadable).toEqual(fixture.unreadable);
});

it("the browser core's read wires match the pinned desktop answers", async () => {
  await initParser();
  const fixture = JSON.parse(readFileSync("tests/fixtures/browser-core-reads.json", "utf8")) as {
    files: [string, string][];
    expected: Record<string, string>;
  };
  const graph = open(fixture.files);
  const uuid = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const targets = JSON.stringify(JSON.parse(fixture.expected.get_backlink_filter_context).entries.map(
    ({ page, kind, block_id }: { page: string; kind: string; block_id: string }) => ({ page, kind, block_id }),
  ));
  const parsed = JSON.parse(fixture.expected.parse_query);
  const query = JSON.stringify(parsed.query);
  const view = JSON.stringify(parsed.view);
  const calls = {
    graph_meta: () => graph.graph_meta_json(),
    page_modified: () => graph.page_modified_json(),
    page_backlink_counts: () => graph.page_backlink_counts_json(),
    resolve_page: () => graph.resolve_page_json("Org Notes", false),
    resolve_namespace: () => graph.resolve_page_json("Team/Child", false),
    page_alias_group: () => graph.page_alias_group_json("Shortcut"),
    journal_content_days: () => graph.journal_content_days_json(),
    page_icons: () => graph.page_icons_json('["Shortcut","Missing"]'),
    get_backlinks: () => graph.get_backlinks_json("Target"),
    get_backlink_filter_context: () => graph.get_backlink_filter_context_json("Target", targets),
    get_unlinked_refs: () => graph.get_unlinked_refs_json("Target"),
    get_block_ref_counts: () => graph.get_block_ref_counts_json(),
    get_block_referrers: () => graph.get_block_referrers_json(uuid),
    resolve_blocks: () => graph.resolve_blocks_json(JSON.stringify([uuid, "missing"])),
    preview_block: () => graph.preview_block_json(uuid, 1),
    search: () => graph.search_json("alpha searchable", 5),
    capture_quick_switch: () => graph.capture_quick_switch_json("Short", 20, false),
    query_facet_keys: () => graph.query_facets_json('{"keys":true}'),
    query_facet_values: () => graph.query_facets_json('{"key":"color"}'),
    parse_query: () => graph.parse_query_json("(task TODO)", "og"),
    query_run: () => graph.query_run_json(query, view),
    run_graph_search: () => graph.run_graph_search_json("alpha", 5, 5, '{"explain":true}'),
  };
  function orderedCounts(wire: string) {
    const [files, references, rev]: [[string, number][], [string, number][], string] = JSON.parse(wire);
    const byKey = ([left]: [string, number], [right]: [string, number]) => left.localeCompare(right);
    return [files.sort(byKey), references.sort(byKey), rev];
  }
  try {
    expect(Object.keys(calls).sort()).toEqual(Object.keys(fixture.expected).sort());
    for (const [name, call] of Object.entries(calls)) {
      // The memory constructor has no source mtimes. The desktop fixture
      // pins its timestamped result with the same graph and adapter.
      const expected = name === "page_modified" ? '{"rev":"0","modified":[]}' : fixture.expected[name];
      const answer = call();
      if (name === "page_backlink_counts") {
        // The wire enumerates page slots. All Pages consumes the counts by key.
        expect.soft(orderedCounts(answer), name).toEqual(orderedCounts(expected));
      } else {
        expect.soft(answer, name).toBe(expected);
      }
      if (process.env.TINE_BROWSER_CORE_TIMINGS === "1") {
        const samples = Array.from({ length: 101 }, () => {
          const start = performance.now();
          call();
          return performance.now() - start;
        }).sort((left, right) => left - right);
        console.log(`${name} ${(samples[50] ?? 0).toFixed(4)} ms`);
      }
    }
    expect(() => graph.query_facets_json("{}")).toThrow(/needs a `key` or `keys: true` scope/);
    expect(() => graph.parse_query_json("(task TODO)", "unknown")).toThrow(/unknown variant/);
    expect(graph.page_icons_json('["Shortcut"]')).toBe(fixture.expected.page_icons);
  } finally {
    graph.free();
  }
});

const encode = (text: string) => new TextEncoder().encode(text);
const open = (files: [string, string][]) =>
  new BrowserGraph(
    files.map(([path]) => path),
    files.map(([, text]) => encode(text)),
  );
const names = (graph: BrowserGraph) =>
  JSON.parse(graph.page_inventory_json()).entries.map((row: { name: string }) => row.name);

// A handle points into the instance that made it: after a fresh instance (a
// trap's recovery) it is refused, and freeing it leaves the new instance alone.
it("a handle from a replaced instance is refused", async () => {
  await initParser();
  const old = open([["pages/old.md", "- old\n"]]);
  __tineReinstantiate();
  const fresh = open([["pages/fresh.md", "- fresh\n"]]);
  expect(() => old.page_inventory_json()).toThrow(/replaced instance/);
  old.free();
  expect(names(fresh)).toEqual(["fresh"]);
  fresh.free();
});

// The constructor refuses what the desktop open refuses, and mismatched arrays.
it("the browser core refuses what the desktop open refuses", async () => {
  await initParser();
  expect(() => new BrowserGraph(["pages/a.md"], [])).toThrow(/1 paths, 0 files/);
  expect(() => open([["/abs.md", "- x\n"]])).toThrow(/graph-relative/);
  expect(() => open([["logseq/config.edn", '{:pages-directory "../out"}']])).toThrow(/pages directory/);
  const graph = open([["logseq/config.edn", "\u00ff"], ["pages/a.md", "- a\n"]]);
  expect(JSON.parse(graph.config_problem_json())).toBeNull();
  graph.free();
  const icloud = open([["logseq/.config.edn.icloud", "x"], ["pages/a.md", "- a\n"]]);
  expect(JSON.parse(icloud.config_problem_json())).toMatchObject({ kind: "config-read" });
  icloud.free();
});
