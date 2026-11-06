// OG-TOAST sweep: a reference rendered before the window's graph is bound
// (right sidebar restored at launch) must not ask the backend for icons and
// raise an error toast for the backend's no-graph refusal.
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { pageIcon, pageIconOwnerPath } from "./pageIconBatch";
import { refreshPageIndex, resetPageIndex } from "./pageIndex";
import { setToasts, toasts } from "./toasts";
import { bumpDataRev } from "./graphSession";
import { createComputed, createRoot } from "solid-js";

it.each([8, 32])("GH #886: one new unloaded icon leaves K unrelated readers asleep, K=%i", async K => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
  const settle = async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); };
  let runs = 0;
  const dispose = createRoot(d => {
    for (let i = 0; i < K; i++) createComputed(() => { runs++; pageIcon(`unloaded-${K}-${i}`); });
    return d;
  });
  try {
    await settle(); runs = 0;
    vi.mocked(backend().pageIcons).mockResolvedValue({ [`single-${K}`]: "*" });
    let shown = "";
    const stopTarget = createRoot(d => { createComputed(() => { shown = pageIcon(`single-${K}`); }); return d; });
    await settle(); stopTarget();
    expect(shown).toBe("*");
    expect(runs).toBe(0);
  } finally { dispose(); }
});

afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

it("an alias resolves relative links from the claimant supplying its icon", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  bumpDataRev();
  const image = "![](../../assets/logo.png)";
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({ Owner: image });
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
    { key: "plain", name: "Plain", is_journal: false, day: null, target: { kind: "existing", id: "pages/Plain.md", others: [] } },
    { key: "owner", name: "Owner", is_journal: false, day: null, target: { kind: "existing", id: "pages/nested/Owner.md", others: [] } },
    { key: "alias", name: "Alias", is_journal: false, day: null, target: { kind: "alias", owners: ["pages/Plain.md", "pages/nested/Owner.md"] } },
  ] });
  await refreshPageIndex();
  try {
    await vi.waitFor(() => expect(pageIconOwnerPath("Alias", image)).toBe("pages/nested/Owner.md"));
  } finally { resetPageIndex(); }
});

it("coalesces visible names and ignores a response from an older publication", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  bumpDataRev();
  let finish!: (icons: Record<string, string>) => void;
  const icons = vi.spyOn(backend(), "pageIcons").mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockResolvedValue({ "Batch A": "AB" });
  pageIcon("Batch A");
  pageIcon("Batch B");
  await vi.waitFor(() => expect(icons).toHaveBeenCalledWith(["Batch A", "Batch B"]));
  bumpDataRev();
  pageIcon("Batch A");
  pageIcon("Batch B");
  await vi.waitFor(() => expect(pageIcon("Batch A")).toBe("AB"));
  finish({ "Batch A": "old" });
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(pageIcon("Batch A")).toBe("AB");
  expect(pageIcon("Batch B")).toBe("");
  expect(icons).toHaveBeenCalledTimes(2);
});

it("an unbound window fetches no page icons and reports nothing; a bound one fetches them", async () => {
  setToasts([]);
  const generation = vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(0);
  const icons = vi.spyOn(backend(), "pageIcons").mockRejectedValue(new Error("missing-graph-binding"));
  expect(pageIcon("Launch Ref")).toBe("");
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(icons).not.toHaveBeenCalled();
  expect(toasts()).toEqual([]);
  generation.mockReturnValue(1);
  icons.mockResolvedValue({ "Launch Ref": "*" });
  pageIcon("Launch Ref");
  await vi.waitFor(() => expect(icons).toHaveBeenCalledWith(["Launch Ref"]));
});

it("a publication keeps a shown icon until its fresh answer replaces or removes it", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  bumpDataRev();
  let finish!: (icons: Record<string, string>) => void;
  const icons = vi.spyOn(backend(), "pageIcons").mockResolvedValueOnce({ "Kept": "K" })
    .mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  pageIcon("Kept");
  await vi.waitFor(() => expect(pageIcon("Kept")).toBe("K"));
  bumpDataRev();
  expect(pageIcon("Kept")).toBe("K");
  await vi.waitFor(() => expect(icons).toHaveBeenCalledTimes(2));
  expect(pageIcon("Kept")).toBe("K");
  finish({});
  await vi.waitFor(() => expect(pageIcon("Kept")).toBe(""));
});
