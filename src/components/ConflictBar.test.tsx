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

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => { resetStore(); host = document.createElement("div"); document.body.append(host); });
afterEach(() => { dispose?.(); host.remove(); vi.restoreAllMocks(); });

it("22a: a live-draft conflict routes to the in-page review and never offers Keep mine (overwrite)", async () => {
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
});

it("22a: a pathless draft's conflict keeps both choices on the bar", async () => {
  loadFeed([{ name: "Nowhere", title: "Nowhere", kind: "page", pre_block: null, rev: null,
    blocks: [{ id: "nowhere-1", raw: "draft", collapsed: false, children: [] }] }]);
  const test = await bindTestHost();
  openAsLoaded(test);
  await conflictedInHost(test, "Nowhere", "pages/Nowhere.md");
  dispose = render(() => <ConflictBar />, host);
  expect([...host.querySelectorAll("button")].map((button) => button.textContent?.trim())).toEqual([
    "Use disk version", "Keep mine (overwrite)",
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
  expect(buttons.map((button) => button.textContent?.trim())).toEqual(["Use disk version", "Keep mine (overwrite)"]);
  expect(host.textContent).toContain("was deleted on disk");
  buttons[0].click();
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
