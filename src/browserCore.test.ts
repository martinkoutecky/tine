import { readFileSync } from "node:fs";
import { beforeAll, expect, it } from "vitest";
import { loadBrowserCore } from "./browserCore";

let core: Awaited<ReturnType<typeof loadBrowserCore>>;
beforeAll(async () => { core = await loadBrowserCore(); });

// The browser core (D-20261122-WEX) answers as the desktop app's store: the
// same graph gives the rows src-tauri's discovery test pins from this fixture.
it("the browser core's page inventory is the desktop app's", () => {
  const fixture = JSON.parse(readFileSync("tests/fixtures/browser-core-inventory.json", "utf8"));
  const graph = open(fixture.files);
  const wire = JSON.parse(graph.page_inventory_json());
  graph.free();
  expect(wire.entries).toEqual(fixture.rows);
  expect(wire.unreadable).toEqual(fixture.unreadable);
});

it("the browser inventory keeps a fileless namespace parent and excludes relative paths and URLs", () => {
  const graph = open([
    ["pages/Team%2FChild.md", "- child\n"],
    ["pages/Dot.md", "title:: .\n- [[./rel/x]] [[../up/y]] [[https://a.b/c]]\n"],
    ["pages/Relative.md", "title:: ./rel/x\n- relative page\n"],
    ["pages/URL.md", "title:: https://a.b/c\n- URL page\n"],
  ]);
  try {
    const { entries } = JSON.parse(graph.page_inventory_json());
    expect(entries).toEqual(expect.arrayContaining([
      expect.objectContaining({
        name: "Team", namespace_parent: true, target: { kind: "absent", id: "pages/Team.md" },
      }),
      ...["./rel/x", "../up/y", "https://a.b/c"].map(name =>
        expect.objectContaining({ name, not_namespace: true })),
    ]));
    for (const name of ["./rel", "../up", "https://a.b", "..", "https:"]) {
      expect(entries).not.toContainEqual(expect.objectContaining({ name, namespace_parent: true }));
    }
  } finally {
    graph.free();
  }
});

it("the browser core's read wires match the pinned desktop answers", () => {
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
    get_page: () => graph.get_page_json("Shortcut", false),
    get_page_by_path: () => graph.get_page_by_path_json("pages/Notes.org"),
    journal_feed_page: () => graph.journal_feed_page_json(1, null, true, null),
    journal_feed_page_before: () => graph.journal_feed_page_json(1, 20260627n, true, null),
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
      } else if (name === "get_page") {
        expect.soft(answer, name).toBe(JSON.stringify(viewPage(JSON.parse(expected))));
      } else if (name === "get_page_by_path") {
        const { page } = JSON.parse(expected);
        expect.soft(answer, name).toBe(JSON.stringify({ page: viewPage(page), source: null }));
      } else if (name === "journal_feed_page" || name === "journal_feed_page_before") {
        const now = new Date();
        const as_of_day = now.getFullYear() * 10000 + (now.getMonth() + 1) * 100 + now.getDate();
        const feed = JSON.parse(expected);
        expect.soft(answer, name).toBe(JSON.stringify({ ...feed, pages: feed.pages.map(viewPage), as_of_day }));
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
    // JSON null for an optional parameter is its default, as on the desktop.
    expect(graph.parse_query_json("(task TODO)", "og", "null")).toBe(fixture.expected.parse_query);
    expect(graph.query_run_json(query, view, "null")).toBe(fixture.expected.query_run);
    const search = graph.run_graph_search_json("alpha", 5, 5, '{"explain":true,"transliteration":null}');
    expect(search).toBe(fixture.expected.run_graph_search);
  } finally {
    graph.free();
  }
});

// A captured physical body carries no save base and cannot be edited.
function viewPage<T extends object>(doc: T) {
  return { ...doc, rev: null, read_only: true };
}

it("the browser core opens the alias owner and can read a different claimant by exact path", () => {
  const graph = open([
    ["pages/A.md", "title:: Same\nalias:: Shortcut\n- owner\n"],
    ["pages/B.md", "title:: Same\nalias:: Shortcut\n- other claimant\n"],
  ]);
  try {
    expect(JSON.parse(graph.get_page_json("Shortcut", false))).toMatchObject({ id: "pages/A.md" });
    expect(JSON.parse(graph.get_page_json("Missing", false))).toBeNull();
    expect(JSON.parse(graph.get_page_by_path_json("pages/B.md"))).toMatchObject({
      page: { id: "pages/B.md", blocks: [{ raw: "other claimant" }] },
      source: null,
    });
    expect(JSON.parse(graph.get_page_by_path_json("pages/b.md"))).toEqual({ page: null, source: null });
  } finally {
    graph.free();
  }
});

it("the browser journal feed extends through a referenced day and includes upcoming references on open", () => {
  const graph = open([
    ["logseq/config.edn", '{:journal/file-name-format "yyyy_dd_MM"}'],
    ["pages/Plan.md", "- [[2000-01-02]] [[2099-01-02]]\n"],
    ["journals/2000_03_01.md", "- recent\n"],
  ]);
  try {
    const files = JSON.parse(graph.journal_feed_page_json(1, null, false, null));
    expect(files.pages.map(({ id }: { id: string }) => id)).toEqual(["journals/2000_03_01.md"]);
    expect(files.references_pending).toBe(false);
    const first = JSON.parse(graph.journal_feed_page_json(1, null, true, 20000102n));
    expect(first.pages.map(({ id }: { id: string }) => id)).toEqual([
      "journals/2000_03_01.md",
      "journals/2000_02_01.md",
    ]);
    expect(first.upcoming).toMatchObject([{ id: "journals/2099_02_01.md", blocks: [] }]);
    const cursor = JSON.parse(graph.journal_feed_page_json(1, 20000103n, true, null));
    expect(cursor.pages).toMatchObject([{ id: "journals/2000_02_01.md", blocks: [] }]);
    expect(cursor.upcoming).toEqual([]);
  } finally {
    graph.free();
  }
});

it("the browser opens on the desktop cursor's day when two journal titles match", () => {
  const graph = open([
    ["logseq/config.edn", '{:journal/page-title-format "yyyy"}'],
    ["pages/2000-01-01.md", "- Jan 1\n"],
    ["pages/2000-01-02.md", "- Jan 2\n"],
  ]);
  try {
    const first = graph.journal_feed_page_json(10, null, true, null);
    expect(first).toBe(graph.journal_feed_page_json(10, 20000103n, true, null));
    expect(JSON.parse(first).pages).toMatchObject([
      { id: "pages/2000-01-01.md", blocks: [{ raw: "Jan 1" }] },
    ]);
  } finally {
    graph.free();
  }
});

const journalMasks = JSON.parse(readFileSync("tests/fixtures/browser-core-journal-masks.json", "utf8")) as {
  name: string;
  files: [string, string][];
  referenced_cursor_rows?: string[];
}[];

it.each(journalMasks)("the browser feed follows the cursor's claimant rules ($name)", ({ files, referenced_cursor_rows }) => {
  const graph = open([
    ...files,
    ["pages/Z.md", "title:: 2000_01_01\n- masked\n"],
    ["journals/1999_12_31.md", "- visible\n"],
    ["pages/Plan.md", "- [[2000-01-01]]\n"],
  ]);
  try {
    for (const before of [null, 20000102n]) {
      for (const references of [false, true]) {
        const feed = JSON.parse(graph.journal_feed_page_json(10, before, references, null));
        const expected = (references && referenced_cursor_rows) || ["journals/1999_12_31.md"];
        expect(feed.pages.map(({ id }: { id: string }) => id)).toEqual(expected);
        expect(feed.done).toBe(true);
      }
    }
  } finally {
    graph.free();
  }
});

it.each([
  ["yy_MM_dd", "20_01_01", "md", "1920-01-01", "2020-01-01", 19200102n, true],
  ["yy_MM_dd", "20_01_01", "org", "1920-01-01", "2020-01-01", 19200102n, true],
  ["yy-MM-dd", "20-01-01", "md", "1920-01-01", "2020-01-01", 19200102n, true],
  ["yy-MM-dd", "20-01-01", "org", "1920-01-01", "2020-01-01", 19200102n, true],
  ["yyyy", "2000", "md", "2000-01-01", "2000-01-02", 20000102n, false],
  ["MM_dd", "01_01", "org", "1920-01-01", "2020-01-01", 19200102n, false],
] as const)("the browser feed reads exact days when %s collides (%s.%s)", (format, stem, ext, oldDay, newDay, before, dated) => {
  const id = `journals/${stem}.${ext}`;
  for (const physical of [false, true]) {
    const files: [string, string][] = [
      ["logseq/config.edn", `{:journal/file-name-format "${format}" :journal/page-title-format "yyyy-MM-dd" :preferred-format :${ext === "md" ? "markdown" : "org"}}`],
      ["pages/Plan.md", `- [[${oldDay}]] [[${newDay}]]\n`],
    ];
    if (physical) files.push([id, ext === "md" ? "- physical\n" : "* physical\n"]);
    const graph = open(files);
    try {
      const older = { id, title: oldDay, blocks: [] };
      const newer = { id, title: newDay, blocks: physical && dated ? [{ raw: "physical" }] : [] };
      const first = JSON.parse(graph.journal_feed_page_json(2, null, true, null));
      expect.soft(first.pages).toMatchObject([newer, older]);
      const cursor = JSON.parse(graph.journal_feed_page_json(2, before, true, null));
      expect.soft(cursor.pages).toMatchObject([older]);
    } finally {
      graph.free();
    }
  }
});

const encode = (text: string) => new TextEncoder().encode(text);
const open = (files: [string, string][]) =>
  new core.BrowserGraph(
    files.map(([path]) => path),
    files.map(([, text]) => encode(text)),
  );
const names = (graph: InstanceType<typeof core.BrowserGraph>) =>
  JSON.parse(graph.page_inventory_json()).entries.map((row: { name: string }) => row.name);

it.each([
  ['; #:demo{:_/hidden ["pages/secret.md"]}', ["public", "secret"]],
  ['#! {:hidden ["pages/secret.md"]}\r', ["public", "secret"]],
  ['#_ #^:meta {:hidden ["pages/secret.md"]}\r\n', ["public", "secret"]],
  // EDN model R4b: a damaged config closes unless every exclusion keyword is
  // an admitted root key, and `:_/hidden` with a torn value is not one.
  ['{:_/hidden ["pages/secret.md"', []],
  ['#:demo{:_/hidden ["pages/secret.md"]}', ["public"]],
  ['^:meta #:demo{:bad #unknown [] :_/hidden ["pages/secret.md"]}', ["public"]],
  ['#^:meta {:hidden ["pages/secret.md"] :bad "bad\\q"}', ["public"]],
  ['^:meta #:demo{:_/hidden ["pages/secret.md"', []],
] as const)("opens with the exclusions authored in config %j", (source, expected) => {
  const graph = open([
    ["logseq/config.edn", source],
    ["pages/public.md", "- Public\n"],
    ["pages/secret.md", "- Secret\n"],
  ]);
  try {
    expect(names(graph)).toEqual(expected);
  } finally {
    graph.free();
  }
});

// A handle points into the instance that made it: after a fresh instance (a
// trap's recovery) it is refused, and freeing it leaves the new instance alone.
it("a handle from a replaced instance is refused", () => {
  const old = open([["pages/old.md", "- old\n"]]);
  core.__tineReinstantiate();
  const fresh = open([["pages/fresh.md", "- fresh\n"]]);
  expect(() => old.page_inventory_json()).toThrow(/replaced instance/);
  old.free();
  expect(names(fresh)).toEqual(["fresh"]);
  // A freed handle is refused too, and freeing it again is a no-op.
  const freed = open([["pages/freed.md", "- freed\n"]]);
  freed.free();
  expect(() => freed.page_inventory_json()).toThrow(/freed handle/);
  expect(() => freed.config_problem_json()).toThrow(/freed handle/);
  freed.free();
  freed[Symbol.dispose]();
  expect(names(fresh)).toEqual(["fresh"]);
  fresh.free();
});

// The constructor refuses what the desktop open refuses, and mismatched arrays.
it("the browser core refuses what the desktop open refuses", () => {
  const { BrowserGraph } = core;
  expect(() => new BrowserGraph(["pages/a.md"], [])).toThrow(/1 paths, 0 files/);
  expect(() => open([["/abs.md", "- x\n"]])).toThrow(/graph-relative/);
  expect(() => open([["logseq/config.edn", '{:pages-directory "../out"}']])).toThrow(/pages directory/);
  const fine = open([["pages/a.md", "- a\n"]]);
  expect(JSON.parse(fine.config_problem_json())).toBeNull();
  fine.free();
  const unreadable = new BrowserGraph(["logseq/config.edn", "pages/a.md"], [new Uint8Array([0xff]), encode("- a\n")]);
  expect(JSON.parse(unreadable.config_problem_json())).toMatchObject({ kind: "config-read" });
  unreadable.free();
  expect(() => open([["pages/a.md", "- a\n"], ["pages/a.md", "- b\n"]])).toThrow(/given twice/);
  expect(() => open([["pages/a.md", "- a\n"], ["pages//a.md", "- b\n"]])).toThrow(/graph-relative/);
  expect(() => open([["x", "x"], ["logseq/config.edn", '{:pages-directory "x/."}']])).toThrow(/not a directory/);
  expect(() => open([["pages/a.md", "x"], ["x/pages/b.md", "x"], ["logseq/config.edn", '{:pages-directory "pages/a.md/p"}']])).toThrow(/under a file/);
  const icloud = open([["logseq/.config.edn.icloud", "x"], ["pages/a.md", "- a\n"]]);
  expect(JSON.parse(icloud.config_problem_json())).toMatchObject({ kind: "config-read" });
  icloud.free();
});

// A browser edit (slice 5a): a submit serializes with the store's one page
// serializer over the current text, replaces the file, and the next read is
// the new text; the version it was typed on is then stale.
const openWith = (files: [string, string][], options: string) =>
  new core.BrowserGraph(
    files.map(([path]) => path),
    files.map(([, text]) => encode(text)),
    options,
  );
const editable = (files: [string, string][]) => openWith(files, JSON.stringify({ editable: true }));

it("a read-only graph takes no edit", () => {
  const graph = open([["pages/Note.md", "- first\n"]]);
  try {
    expect(() => graph.replace_files(["pages/Note.md"], [encode("- changed\n")])).toThrow(/editable/);
    const page = JSON.parse(graph.get_page_json("Note", false));
    page.blocks[0].raw = "edited";
    const answer = JSON.parse(graph.submit_page_json("pages/Note.md", JSON.stringify(page), 1n));
    expect(answer).toMatchObject({ refused: { reason: "failed" } });
    expect(JSON.parse(graph.get_page_json("Note", false))).toMatchObject({ blocks: [{ raw: "first" }] });
  } finally {
    graph.free();
  }
});

it("only null deletes a replaced file", () => {
  const graph = editable([["pages/Note.md", "- first\n"]]);
  try {
    expect(() => graph.replace_files(["pages/Note.md"], [undefined])).toThrow(/not bytes or null/);
    expect(() => graph.replace_files(["pages/Note.md"], new Array(1))).toThrow(/not bytes or null/);
    expect(names(graph)).toContain("Note");
    JSON.parse(graph.replace_files(["pages/Note.md"], [null]));
    expect(names(graph)).not.toContain("Note");
  } finally {
    graph.free();
  }
});

it("a pre-epoch file time is no recency row", () => {
  const graph = openWith(
    [["pages/A.md", "- a\n"], ["pages/B.md", "- b\n"], ["pages/C.md", "- c\n"]],
    '{"times":{"pages/A.md":-1,"pages/B.md":-9223372036854775808,"pages/C.md":5}}',
  );
  try {
    expect(JSON.parse(graph.page_modified_json()).modified).toEqual([["pages/C.md", 5]]);
  } finally {
    graph.free();
  }
});

it("a share is never editable, whatever the options say", () => {
  const share = { share: { name: "Shared", exported_at: "2026-11-23T00:00:00Z" }, editable: true };
  const graph = openWith([["pages/Note.md", "- first\n"]], JSON.stringify(share));
  try {
    expect(() => graph.replace_files(["pages/Note.md"], [encode("- changed\n")])).toThrow(/editable/);
    const page = JSON.parse(graph.get_page_json("Note", false));
    page.blocks[0].raw = "edited";
    const answer = JSON.parse(graph.submit_page_json("pages/Note.md", JSON.stringify(page), 1n));
    expect(answer).toMatchObject({ refused: { reason: "failed" } });
    expect(JSON.parse(graph.get_page_json("Note", false))).toMatchObject({ blocks: [{ raw: "first" }] });
  } finally {
    graph.free();
  }
});

it("a time for no file is refused, whatever its sign", () => {
  for (const path of ["pages/None.md", "assets/none.png", "logseq/config.edn"]) {
    for (const time of ["-1", "-9223372036854775808", "0", "5"]) {
      expect(() => openWith([["pages/A.md", "- a\n"]], `{"times":{"${path}":${time}}}`), `${path} ${time}`)
        .toThrow(/a time for no file/);
    }
  }
});

it("a submitted page is read back as its new text", () => {
  const graph = editable([["pages/Note.md", "- first\n- second\n"]]);
  try {
    const page = JSON.parse(graph.get_page_json("Note", false));
    page.blocks[0].raw = "edited";
    const answer = JSON.parse(graph.submit_page_json("pages/Note.md", JSON.stringify(page), 1n));
    expect(answer).toMatchObject({
      written: { path: "pages/Note.md", text: "- edited\n- second\n", version: 2 },
    });
    expect(answer.written.pages).toEqual([
      { path: "pages/Note.md", name: "Note", kind: "page", created: false, removed: false },
    ]);
    expect(JSON.parse(graph.get_page_json("Note", false))).toMatchObject({
      id: "pages/Note.md",
      blocks: [{ raw: "edited" }, { raw: "second" }],
    });
    const stale = JSON.parse(graph.submit_page_json("pages/Note.md", JSON.stringify(page), 1n));
    expect(stale).toEqual({ refused: { reason: "not-admitted" } });
  } finally {
    graph.free();
  }
});

// The preview's exact delete: the core decides it under the submit's stale rule.
it("deletes a page file only at the version the core holds", () => {
  const graph = editable([["pages/Note.md", "- first\n"]]);
  try {
    expect(graph.version("pages/Note.md")).toBe(1n);
    const page = JSON.parse(graph.get_page_json("Note", false));
    graph.submit_page_json("pages/Note.md", JSON.stringify(page), 1n);
    expect(graph.version("pages/Note.md")).toBe(2n);
    expect(JSON.parse(graph.delete_file_json("pages/Note.md", 1n))).toEqual({ refused: { reason: "not-admitted" } });
    expect(JSON.parse(graph.get_page_json("Note", false))).toMatchObject({ id: "pages/Note.md" });
    expect(JSON.parse(graph.delete_file_json("pages/Note.md", 2n)).deleted.pages).toEqual([
      { path: "pages/Note.md", name: "Note", kind: "page", created: false, removed: true },
    ]);
    expect(graph.version("pages/Note.md")).toBe(3n);
    expect(JSON.parse(graph.delete_file_json("pages/Note.md", 3n))).toEqual({ refused: { reason: "not-admitted" } });
  } finally {
    graph.free();
  }
});

// The preview's delete by name takes the desktop's one target.
it("deletes by name only the name's one claimant", () => {
  const graph = editable([["pages/Same.md", "- md\n"], ["pages/Same.org", "* org\n"], ["pages/One.md", "- one\n"]]);
  try {
    expect(JSON.parse(graph.delete_page_json("Same", false))).toMatchObject({ refused: { reason: "invalid-target" } });
    expect(JSON.parse(graph.delete_page_json("One", false, "pages/Same.md"))).toMatchObject({ refused: { reason: "invalid-target" } });
    expect(JSON.parse(graph.delete_page_json("Absent", false))).toEqual({ deleted: null });
    expect(JSON.parse(graph.delete_page_json("Absent", false, "pages/One.md"))).toMatchObject({ refused: { reason: "invalid-target" } });
    expect(JSON.parse(graph.delete_page_json("One", false, "pages/One.md")).deleted).toBeTruthy();
    expect(JSON.parse(graph.get_page_by_path_json("pages/One.md")).page).toBeNull();
    expect(JSON.parse(graph.get_page_by_path_json("pages/Same.md")).page).toBeTruthy();
  } finally {
    graph.free();
  }
});
