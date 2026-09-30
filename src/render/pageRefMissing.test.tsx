import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetPageIndex, refreshPageIndex } from "../pageIndex";
import { bumpPageInventoryRev } from "../graphSession";
import type { PageInventory, ResolvedPage } from "../types";
import { PageRef } from "./inline";
import { initParser } from "./parse";
import { openPage } from "../router";
vi.mock("../router", async (original) => ({ ...await original<typeof import("../router")>(), openPage: vi.fn() }));
beforeAll(() => initParser());
afterEach(() => { resetPageIndex(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it("missing refs and tags restyle on alias/create/delete inventory changes without per-link IPC (I-12/I-25)", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  const absent: ResolvedPage = { kind: "absent", id: "pages/New.md" };
  const existing: ResolvedPage = { kind: "existing", id: "pages/filename.md", others: [] };
  const inventory = (rev: number, target: ResolvedPage): PageInventory => ({ rev: String(rev), entries: [
    { key: "new", name: "New", is_journal: false, day: null, target },
    { key: "café", name: "Café", is_journal: false, day: null, target: existing },
  ] });
  const read = vi.spyOn(backend(), "pageInventory").mockResolvedValue(inventory(1, absent));
  await refreshPageIndex();
  const host = document.createElement("div"); document.body.appendChild(host);
  const dispose = render(() => <><PageRef name="New" /><PageRef name="New" tag />
    <PageRef name="CAFÉ" alias="label" /><PageRef name="Tine-guide/Tine Guide" /></>, host);
  try {
    const links = host.querySelectorAll("a");
    await vi.waitFor(() => expect(links[0].classList.contains("page-ref-missing")).toBe(true));
    expect(links[1].classList.contains("page-ref-missing")).toBe(true);
    expect(links[2].classList.contains("page-ref-missing")).toBe(false);
    expect(links[3].classList.contains("page-ref-missing")).toBe(false);
    const calls = read.mock.calls.length;
    links[0].dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(openPage).toHaveBeenCalledWith("New", "page");
    for (const [rev, target, missing] of [
      [2, { kind: "alias", owners: ["pages/filename.md"] }, false],
      [3, absent, true], [4, existing, false], [5, absent, true],
    ] as const) {
      read.mockResolvedValue(inventory(rev, target as ResolvedPage));
      bumpPageInventoryRev();
      await vi.waitFor(() => expect(links[0].classList.contains("page-ref-missing")).toBe(missing));
      expect(links[1].classList.contains("page-ref-missing")).toBe(missing);
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(calls + rev - 1));
    }
    expect(host.querySelectorAll("a")).toHaveLength(4);
  } finally { dispose(); }
});
