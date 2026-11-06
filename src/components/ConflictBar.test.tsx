import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ConflictBar } from "./ConflictBar";
import { conflicts, isDirty, loadFeed, node, pageByName, resetStore } from "../document";
import { backend } from "../backend";
import { endEdit } from "../editorController";
import { installRouterBridge } from "../routerBridge";
import type { PageTarget } from "../routeTypes";
import { bindTestHost, mailPage } from "../document/host/wiring.test.support";
import { baseRevFor } from "../document/host/wiring";
import { conflictedInHost, openAsLoaded, openInHost, reportConflict } from "./hostConflict.test.support";
import { conflictReviewRequest } from "../liveConflicts";
import { ConflictOverview } from "./ConflictOverview";
import { ConflictQueueBadge } from "./Sidebar";
import type { PaneRouter } from "../router";

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => { resetStore(); host = document.createElement("div"); document.body.append(host); });
afterEach(() => { dispose?.(); host.remove(); vi.restoreAllMocks(); });

it.each(["md", "org", "journal"] as const)("GH #875: host retirement removes every conflict surface for %s", async (format) => {
  const kind = format === "journal" ? "journal" : "page";
  const id = `${kind === "journal" ? "journals" : "pages"}/Live.${format === "org" ? "org" : "md"}`;
  loadFeed([{ id, name: "Live", title: "Live", kind, format: format === "org" ? "org" : "md", pre_block: null, rev: "r1",
    blocks: [{ id: "live-1", raw: "mine", collapsed: false, children: [] }] }]);
  vi.spyOn(backend(), "conflictInventory").mockResolvedValue({ sync_conflicts: [], vcs_markers: [], queue: [] });
  const test = await bindTestHost();
  openAsLoaded(test);
  await conflictedInHost(test, "Live");
  const router = { openPageTarget: vi.fn() } as unknown as PaneRouter;
  dispose = render(() => <><ConflictBar /><ConflictOverview router={router} /><ConflictQueueBadge /></>, host);
  await vi.waitFor(() => expect(host.querySelectorAll(".conflict-overview-row")).toHaveLength(1));
  expect(host.querySelector(".conflict-banner")).not.toBeNull();
  expect(host.querySelector(".conflict-queue-badge")).not.toBeNull();
  test.deliver({ key: id, answer: null, page: {
    version: 2, conflict: false, risk: true, disk: { kind: "file", rev: "identical" }, text: { kind: "unchanged" },
  } });
  await vi.waitFor(() => expect(conflicts()).toEqual([]));
  expect(host.querySelector(".conflict-banner")).toBeNull();
  expect(host.querySelector(".conflict-overview-row")).toBeNull();
  expect(host.querySelector(".conflict-queue-badge")).toBeNull();
  expect(node("live-1")?.raw).toBe("mine");
});

it("22a: a live-draft conflict routes to the in-page review and never offers Keep mine", async () => {
  loadFeed([{ id: "pages/Live.md", name: "Live", title: "Live", kind: "page", pre_block: null, rev: "r1",
    blocks: [{ id: "live-1", raw: "mine", collapsed: false, children: [] }] }]);
  const test = await bindTestHost();
  openAsLoaded(test);
  await conflictedInHost(test, "Live");
  const opened: PageTarget[] = [];
  installRouterBridge({ route: () => ({ kind: "journals" }), focusBlock: () => {}, scheduleSessionSave: () => {}, openPageTarget: (t) => opened.push(t) });
  dispose = render(() => <ConflictBar />, host);
  const buttons = [...host.querySelectorAll("button")];
  expect(buttons.map((button) => button.textContent?.trim())).toEqual(["Review"]);
  buttons[0].click();
  expect(opened).toEqual([{ name: "Live", pageKind: "page", path: "pages/Live.md" }]);
  // GH #858: opening the page already open is a no-op, so Review also asks
  // that page's resolver to come into view.
  expect(conflictReviewRequest()?.path).toBe("pages/Live.md");
});

it("22a: a pathless draft's conflict keeps both choices on the bar", async () => {
  loadFeed([{ name: "Nowhere", title: "Nowhere", kind: "page", pre_block: null, rev: null,
    blocks: [{ id: "nowhere-1", raw: "draft", collapsed: false, children: [] }] }]);
  const test = await bindTestHost();
  openAsLoaded(test);
  await conflictedInHost(test, "Nowhere", "pages/Nowhere.md");
  dispose = render(() => <ConflictBar />, host);
  expect([...host.querySelectorAll("button")].map((button) => button.textContent?.trim())).toEqual([
    "Keep mine", "Use disk version",
  ]);
});

it("GH #541: a conflict whose file was deleted on disk offers the whole-page choices, and Use disk version accepts the deletion", async () => {
  loadFeed([{ id: "pages/Gone.md", name: "Gone", title: "Gone", kind: "page", pre_block: null, rev: "r1",
    blocks: [{ id: "gone-1", raw: "mine", collapsed: false, children: [] }] }]);
  const test = await bindTestHost();
  openAsLoaded(test);
  await openInHost("Gone");
  reportConflict(test, "pages/Gone.md", null);
  endEdit("blur");
  const discard = vi.spyOn(backend(), "pageDiscard").mockImplementation(async (_session, id, key) => {
    queueMicrotask(() => test.deliver({ key, answer: { id, version: 3, took: true, outcome: { kind: "applied" } },
      page: mailPage(3, null) }));
    return null;
  });
  dispose = render(() => <ConflictBar />, host);
  const buttons = [...host.querySelectorAll("button")];
  // A review against "the file on disk now" has no file to open (the page
  // route fails on the missing path): both whole-page choices stay here.
  expect(buttons.map((button) => button.textContent?.trim())).toEqual(["Keep mine", "Use disk version"]);
  expect(host.textContent).toContain("was deleted on disk");
  buttons[1].click();
  await vi.waitFor(() => expect(conflicts()).toEqual([]));
  expect(discard).toHaveBeenCalledOnce();
  // The page is what a missing file shows: no file, none of the dropped text,
  // nothing left to save (so nothing recreates the deleted file).
  const page = pageByName("Gone");
  expect(baseRevFor("Gone")).toBeNull();
  expect(page?.roots.map((id) => node(id)?.raw)).not.toContain("mine");
  expect(isDirty("Gone")).toBe(false);
});

it("GH #541: Keep mine on a deleted file's conflict recreates it from the window's text, on the proved missing file", async () => {
  loadFeed([{ id: "pages/Gone.md", name: "Gone", title: "Gone", kind: "page", pre_block: null, rev: "r1",
    blocks: [{ id: "gone-1", raw: "mine", collapsed: false, children: [] }] }]);
  const test = await bindTestHost();
  openAsLoaded(test);
  await openInHost("Gone");
  reportConflict(test, "pages/Gone.md", null);
  endEdit("blur");
  const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => {
    queueMicrotask(() => test.deliver({ key, answer: { id, version: 3, took: true, outcome: { kind: "applied" } },
      page: { version: 3, conflict: false, risk: true, disk: { kind: "no-file" }, text: { kind: "unchanged" } } }));
    return null;
  });
  dispose = render(() => <ConflictBar />, host);
  [...host.querySelectorAll("button")].find((button) => button.textContent?.includes("Keep mine"))!.click();
  await vi.waitFor(() => expect(conflicts()).toEqual([]));
  expect(submit).toHaveBeenCalledOnce();
  const [, , key, dto, , resolve] = submit.mock.calls[0];
  expect([key, resolve]).toEqual(["pages/Gone.md", { kind: "no-file" }]);
  expect(dto.blocks.map((block) => block.raw)).toEqual(["mine"]);
});
