import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { applyChangeSummary, setChangeSummaryBaseline } from "../changeSummary";
import { TagTableToggle } from "./Page";
import { resetStore } from "../document";

let dispose: (() => void) | undefined;
beforeEach(() => {
  resetStore(); bumpGraphEpoch();
  setChangeSummaryBaseline("0");
});
afterEach(() => { dispose?.(); dispose = undefined; vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("opening a page requests only availability, ignores unrelated body changes and follows graph switches", async () => {
  const available = vi.spyOn(backend(), "tagTableAvailable").mockResolvedValue(true);
  const parse = vi.spyOn(backend(), "parseQuery");
  const groups = vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "block", groups: [{ page: "Source", kind: "page", blocks: [{ id: "row", raw: "#Target", children: [] }] }], exceeded: false } as never);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <TagTableToggle page={{ name: "Target", kind: "page", roots: [] } as never} />, root);
  await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).not.toBeNull());
  expect(available).toHaveBeenCalledExactlyOnceWith("Target");
  expect(parse).not.toHaveBeenCalled();
  expect(groups).not.toHaveBeenCalled();
  applyChangeSummary({ rev: "1", full: false, pages: ["unrelated"], blocks: ["shown"],
    linksTo: [], textTo: [], tagsTo: [], tagTableTo: ["target"], tagTableMembershipTo: [], refsTo: [], props: {}, inventoryChanged: false });
  bumpDataRev(true);
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(available).toHaveBeenCalledTimes(1);
  available.mockResolvedValue(false);
  bumpGraphEpoch();
  await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).toBeNull());
  expect(available).toHaveBeenCalledTimes(2);
  expect(groups).not.toHaveBeenCalled();
});

it("adopts membership removal without fetching groups", async () => {
  const available = vi.spyOn(backend(), "tagTableAvailable").mockResolvedValue(true);
  vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "block", groups: [{ page: "Source", kind: "page", blocks: [{ id: "row", raw: "#Target", children: [] }] }], exceeded: false } as never);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <TagTableToggle page={{ name: "Target", kind: "page", roots: [] } as never} />, root);
  await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).not.toBeNull());
  available.mockResolvedValue(false);
  applyChangeSummary({ rev: "1", full: false, pages: [], blocks: [], linksTo: [], textTo: [], tagsTo: [],
    tagTableTo: [], tagTableMembershipTo: ["target"], refsTo: [], props: {}, inventoryChanged: false });
  bumpDataRev(true);
  await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).toBeNull());
  expect(available).toHaveBeenCalledTimes(2);
});

it("a late answer for the previous page cannot hide the current page's Table button", async () => {
  const pending = new Map<string, (answer: boolean) => void>();
  vi.spyOn(backend(), "tagTableAvailable").mockImplementation(name => new Promise(resolve => pending.set(name, resolve)));
  const [name, setName] = createSignal("Target");
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <TagTableToggle page={{ name: name(), kind: "page", roots: [] } as never} />, root);
  await vi.waitFor(() => expect(pending.has("Target")).toBe(true));
  setName("Other");
  await vi.waitFor(() => expect(pending.has("Other")).toBe(true));
  pending.get("Other")!(true);
  await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).not.toBeNull());
  pending.get("Target")!(false);
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(root.querySelector(".tag-table-toggle")).not.toBeNull();
});
