import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { loadGraphPath } from "./graph";
import { flushPage, pageByName, resetStore, setRaw } from "./document";
import { loadSingle } from "./document/workingSet";
import { setGraphMeta } from "./graphSession";
import { submittedPages } from "./document/host/wiring.test.support";
import { setToasts, toasts } from "./toasts";
import type { DraftStatus } from "./document/host/protocol";
import type { GraphMeta } from "./types";

// Crash drafts belong to the page host (STEP3 §9). The window's part is to say
// what the host could not use, never to block the open over it (B-Q1): an
// unreadable draft file stays in place and is named; storage the host cannot
// use turns crash recovery off with a Retry while edits keep saving; a v1 draft
// file stays untouched as a backup and is named once (D-1).

const ROOT = "/tmp/draft-status-graph";
const meta = (): GraphMeta => ({
  root: ROOT, journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now", shortcuts: {},
  start_of_week: 6, block_hidden_properties: [], linked_references_collapsed_threshold: 100, default_journal_template: null, favorites: [],
  journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd", preferred_format: "md",
  macros: {}, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
});

function openGraph(draftStatus: DraftStatus | null) {
  const api = backend();
  vi.spyOn(api, "inspectGraphAccess").mockResolvedValue({ graph_root: ROOT, external_assets_path: null, approved: true });
  vi.spyOn(api, "loadGraph").mockResolvedValue({ kind: "loaded", meta: meta(), binding_generation: 1, draft_status: draftStatus });
  vi.spyOn(api, "getPage").mockResolvedValue(null);
  vi.spyOn(api, "readCustomCss").mockResolvedValue("");
  vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
  return loadGraphPath(ROOT);
}

const errors = () => toasts().filter((t) => t.kind === "error");
const said = (text: string) => toasts().filter((t) => t.message.includes(text));

beforeEach(() => { resetStore(); setGraphMeta(null); setToasts([]); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); setToasts([]); localStorage.clear(); });

describe("crash-recovery status at graph open (STEP3 §9, B-Q1)", () => {
  it("unreadable crash-recovery files are named in a sticky error, and the graph still opens", async () => {
    const result = await openGraph({ unreadable: ["P.draft", "Q.draft"] });
    expect(result.kind).toBe("loaded");
    const [toast] = said("unreadable crash-recovery file");
    expect(toast).toMatchObject({ kind: "error", sticky: true,
      message: "Tine left 2 unreadable crash-recovery file(s) in place: P.draft, Q.draft." });
  });

  it("storage the host cannot use turns crash recovery off with a Retry; the graph opens and edits still save", async () => {
    const submit = vi.spyOn(backend(), "pageSubmit");
    const result = await openGraph({ unavailable: "drafts directory: permission denied", unreadable: [] });
    expect(result.kind).toBe("loaded");
    const [off] = said("Crash recovery is off");
    expect(off).toMatchObject({ kind: "error", sticky: true,
      message: "Crash recovery is off for this graph: drafts directory: permission denied. Your edits still save." });
    expect(off.action?.label).toBe("Retry");

    loadSingle({ id: "pages/P.md", name: "P", title: "P", kind: "page", pre_block: null, rev: "r1",
      blocks: [{ id: "b", raw: "before", collapsed: false, children: [] }] });
    setRaw(pageByName("P")!.roots[0], "after");
    expect(await flushPage("P")).toBe(true);
    expect(submittedPages(submit, "P").at(-1)?.blocks[0].raw).toBe("after");

    const retry = vi.spyOn(backend(), "pageDraftsRetry");
    off.action!.run();
    await vi.waitFor(() => expect(said("Crash recovery is on again.")).toHaveLength(1));
    expect(retry).toHaveBeenCalledOnce();
    expect(said("Crash recovery is off")).toHaveLength(0);
  });

  it("a Retry that still fails stays a sticky error and offers a restart on desktop", async () => {
    await openGraph({ unavailable: "drafts directory: permission denied", unreadable: [] });
    vi.spyOn(backend(), "pageDraftsRetry").mockRejectedValue("still denied");
    said("Crash recovery is off")[0].action!.run();
    await vi.waitFor(() => expect(said("Crash recovery is still off")).toHaveLength(1));
    const [still] = said("Crash recovery is still off");
    expect(still).toMatchObject({ kind: "error", sticky: true, message: "Crash recovery is still off: still denied." });
    expect(still.action?.label).toBe("Restart Tine");
  });

  it("pages launch recovered from crash-recovery copies are named in a sticky notice (STEP3 §9)", async () => {
    await openGraph({ unreadable: [], unsaved: [
      { path: "pages/P.md", recovered: true, conflict: false, failing: false },
      { path: "pages/Q.md", recovered: true, conflict: false, failing: false },
    ] });
    expect(said("Recovered unsaved edits")).toEqual([expect.objectContaining({ kind: "info", sticky: true,
      message: "Recovered unsaved edits on 2 page(s) — Tine is saving them: pages/P.md, pages/Q.md." })]);
    expect(errors()).toEqual([]);
  });

  it("pages a kept host still holds are named with why, as an error when one needs the user (B-QA)", async () => {
    await openGraph({ unreadable: [], unsaved: [
      { path: "pages/P.md", recovered: false, conflict: true, failing: false },
      { path: "pages/Q.md", recovered: false, conflict: false, failing: true },
      { path: "pages/R.md", recovered: false, conflict: false, failing: false },
    ] });
    expect(said("not on disk")).toEqual([expect.objectContaining({ kind: "error", sticky: true,
      message: "3 page(s) still have edits that are not on disk; Tine keeps saving them: "
        + "pages/P.md (changed on disk; open it to choose), pages/Q.md (saving fails), pages/R.md." })]);
  });

  it("a usable draft store says nothing", async () => {
    await openGraph({ unreadable: [] });
    expect(errors()).toEqual([]);
  });

  it("the v1 crash-draft file is named once as an untouched backup, not again when the same graph reopens (D-1)", async () => {
    const legacy = vi.spyOn(backend(), "legacyDraftsFile")
      .mockResolvedValueOnce(`${ROOT}/.tine/drafts.json`)
      .mockResolvedValueOnce(`${ROOT}/.tine/drafts.json`)
      .mockResolvedValueOnce(`${ROOT}/.tine/other.json`);
    await openGraph(null);
    await vi.waitFor(() => expect(said("from an older Tine")).toHaveLength(1));
    expect(said("from an older Tine")[0]).toMatchObject({ kind: "info",
      message: `Crash-recovery copies from an older Tine are kept untouched in ${ROOT}/.tine/drafts.json.` });
    setToasts([]);
    await loadGraphPath(ROOT, { forceRefresh: true });
    // A third open naming another file is the anchor: once it is said, the
    // reopen's check (started first, same steps) has finished in silence.
    await loadGraphPath(ROOT, { forceRefresh: true });
    await vi.waitFor(() => expect(said("from an older Tine")).toHaveLength(1));
    expect(legacy).toHaveBeenCalledTimes(3);
    expect(said("from an older Tine")[0].message).toContain("other.json");
  });
});
