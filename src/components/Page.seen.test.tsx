// "Changed since you last looked" on the routed page (vision 9a, ADR 0073):
// an untracked page renders exactly as before, and after Mark seen an edit to
// one block highlights exactly that block and counts it.
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend, type Backend } from "../backend";
import { resetStore, setRaw } from "../document";
import { setGraphMeta } from "../graphSession";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetSeenBaselinesForTests, markPageSeen } from "../seen/baseline";
import { PageView } from "./Page";

const blocks = ["Alpha", "Beta\nstatus:: open", "Gamma"].map((raw, i) => ({
  id: `seen-${i}`, has_id: false, raw, children: [], collapsed: false,
}));

function openPage() {
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/Seen.md", name: "Seen", title: "Seen", kind: "page", pre_block: null, blocks: structuredClone(blocks),
  });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageView />, host);
  return { host, dispose };
}
const rowOf = (host: Element, id: string) => host.querySelector(`[data-block-id="${id}"] > .block-main`);

let saved: Backend["seenBaseline"];
beforeEach(() => {
  saved = backend().seenBaseline;
  resetSeenBaselinesForTests();
  setGraphMeta({ root: "/graphs/seen" } as never);
});
afterEach(() => {
  (backend() as { seenBaseline?: Backend["seenBaseline"] }).seenBaseline = saved;
  vi.restoreAllMocks();
  resetStore();
  resetTabsToJournals();
  setGraphMeta(null);
  document.body.replaceChildren();
});

it("vision 9a: an untracked page renders exactly as on a backend that keeps no seen state", async () => {
  const records = new Map<string, string[]>();
  const io = vi.fn(async (r: { op: string; page: string }) => records.get(r.page) ?? null);
  (backend() as { seenBaseline?: unknown }).seenBaseline = io;
  const tracked = openPage();
  await vi.waitFor(() => expect(tracked.host.querySelectorAll(".ls-block")).toHaveLength(3));
  await vi.waitFor(() => expect(io).toHaveBeenCalledWith(expect.objectContaining({ op: "load" })));
  await Promise.resolve();
  const withSeen = tracked.host.innerHTML;
  tracked.dispose();
  resetStore();
  document.body.replaceChildren();

  delete (backend() as { seenBaseline?: unknown }).seenBaseline;
  const plain = openPage();
  await vi.waitFor(() => expect(plain.host.querySelectorAll(".ls-block")).toHaveLength(3));
  expect(withSeen).toBe(plain.host.innerHTML);
  expect(plain.host.querySelector(".seen-changed, [data-seen-header]")).toBeNull();
  plain.dispose();
});

it("vision 9a: after Mark seen, an edit to one block highlights exactly that block and counts one change", async () => {
  const { host, dispose } = openPage();
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".ls-block")).toHaveLength(3));
    expect(await markPageSeen("Seen")).toBe(true);
    await Promise.resolve();
    expect(host.querySelectorAll(".seen-changed")).toHaveLength(0);
    expect(host.querySelector("[data-seen-header]")).toBeNull();

    setRaw("seen-1", "Beta\nstatus:: done");
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    const changed = [...host.querySelectorAll(".block-main.seen-changed")];
    expect(changed).toHaveLength(1);
    expect(changed[0]).toBe(rowOf(host, "seen-1"));
    expect(host.querySelector("[data-seen-header]")?.textContent).toContain("1 change since you last looked");

    setRaw("seen-2", "Gamma edited");
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")?.textContent).toContain("2 changes since you last looked"));
    expect(host.querySelectorAll(".block-main.seen-changed")).toHaveLength(2);

    // Mark seen from the header clears both.
    (host.querySelector(".seen-header-mark") as HTMLButtonElement).click();
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).toBeNull());
    expect(host.querySelectorAll(".seen-changed")).toHaveLength(0);
  } finally { dispose(); }
});
