// GH #855: a journal section renders its query on the journal feed away from
// its block on the sections page. The View settings the user chose there are
// `tine.*` properties of that block (sort, group), and the feed must run the
// query with them, exactly as the sections page does. The feed only shows: it
// has no block of its own to write a View change into.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QueryMacro } from "./Macro";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import type { ParsedQuery, ViewSettings } from "../editor/queryIr";
import type { RefGroup } from "../types";

beforeAll(initParser);

afterEach(() => {
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.innerHTML = "";
});

const SECTION = "{{query (task TODO)}}\ntine.sort:: priority asc\ntine.group-field:: tags\nnotes:: not a view setting";

function groups(): RefGroup[] {
  return [{ page: "Work", kind: "page", blocks: [
    { id: "a", raw: "TODO first row", collapsed: false, children: [] },
    { id: "b", raw: "TODO second row", collapsed: false, children: [] },
  ] }];
}

/** The engine reads the view out of the host block's `tine.*` properties. */
function engineReadsHostProperties(extra: Partial<ParsedQuery> = {}) {
  const parse = vi.spyOn(backend(), "parseQuery").mockImplementation(async (_text, _dialect, properties) => {
    const view: ViewSettings = {};
    for (const [key, value] of properties ?? []) {
      if (key === "tine.sort") view.sort = [[value.split(" ")[0], value.endsWith("desc") ? "desc" : "asc"]];
      if (key === "tine.group-field") view.group_by = value;
    }
    return { query: { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: "(task TODO)", og_options: "" } }, view, ...extra };
  });
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  return { parse, run };
}

function mount(raw: string) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <QueryMacro body="query (task TODO)" currentPage="Nov 3rd, 2026" hideWhenEmpty
    hostRaw={{ raw, format: "md" }} />, root);
  return { root, dispose };
}

it("the feed runs a section's query with the View properties of its block (GH #855)", async () => {
  const { parse, run } = engineReadsHostProperties();
  const { root, dispose } = mount(SECTION);
  try {
    await vi.waitFor(() => expect(root.querySelector(".query-count")?.textContent).toBe("2"));
    expect(parse.mock.calls.at(-1)?.[2]).toEqual([["tine.sort", "priority asc"], ["tine.group-field", "tags"]]);
    const view = run.mock.calls.at(-1)?.[1] as ViewSettings;
    expect(view.group_by).toBe("tags");
    expect(view.sort).toEqual([["priority", "asc"]]);
  } finally { dispose(); }
});

it.each(["table", "board"])("a section shown as %s on its page still lists its rows on the feed", async face => {
  engineReadsHostProperties({ block_presentation: face as ParsedQuery["block_presentation"] });
  const { root, dispose } = mount(`${SECTION}\ntine.view:: ${face}`);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("first row"));
    expect(root.textContent).toContain("second row");
  } finally { dispose(); }
});
