import { afterEach, expect, it, vi } from "vitest";
import { batch, For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, resetStore, setPageProperty } from "../document";
import * as icons from "../pageIconBatch";
import { PageRef } from "./inline";
import { installPageIndex, linkTarget, patchPageIndex, resetPageIndex } from "../pageIndex";
import { openPage, route } from "../router";
import type { PageInventoryEntry } from "../types";

afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it("updates a mounted alternate-date link after the canonical day's alias owner changes", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
  const day = (owner: string): PageInventoryEntry => ({ key: "oct 23rd, 2026", name: "Oct 23rd, 2026",
    is_journal: true, day: 20261023, target: { kind: "alias", owners: [`pages/${owner}.md`] },
    link: { name: owner, kind: "page" } });
  resetPageIndex();
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [day("Alpha"),
    { key: "alpha", name: "Alpha", is_journal: false, day: null, target: { kind: "absent", id: "pages/Alpha.md" } },
    { key: "beta", name: "Beta", is_journal: false, day: null, target: { kind: "existing", id: "pages/Beta.md", others: [] } },
  ] });
  installPageIndex();
  await vi.waitFor(() => expect(linkTarget("2026-10-23").name).toBe("Alpha"));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <PageRef name="2026-10-23" />, root);
  try {
    const anchor = root.querySelector<HTMLAnchorElement>("a.page-ref")!;
    expect(anchor.hasAttribute("data-missing-page")).toBe(true);
    expect(patchPageIndex(2n, { keys: ["oct 23rd, 2026"], entries: [day("Beta")] })).toEqual({ aliases: true });
    expect(anchor.hasAttribute("data-missing-page")).toBe(false);
    openPage("Before");
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(route()).toMatchObject({ kind: "page", name: "Beta" });
  } finally {
    dispose();
    resetPageIndex();
  }
});

it("resolves newly loaded names inside an enclosing reactive batch, including prototype-like names", () => {
  batch(() => {
    for (const name of ["Fresh batch page", "constructor", "__proto__", "toString"]) {
      expect(pageByName(name)).toBeUndefined();
      ensurePageLoaded({ name, title: name, id: `pages/${name}.md`, kind: "page", pre_block: "", format: "md", blocks: [] });
      expect(pageByName(name)?.id).toBe(`pages/${name}.md`);
    }
  });
  resetStore();
  expect(pageByName("constructor")).toBeUndefined();
});

it("loading unrelated sources does not reevaluate retained property-link icons, while the target's live icon still updates", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
  const icon = vi.spyOn(icons, "pageIcon");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <For each={Array.from({ length: 200 }, (_, i) => i)}>{() => <PageRef name="Property target" />}</For>, root);
  const page = (name: string, pre_block = "") => ({ name, title: name, id: `pages/${name}.md`, kind: "page" as const, pre_block, format: "md" as const, blocks: [] });
  try {
    await Promise.resolve();
    expect(root.querySelectorAll(".page-ref")).toHaveLength(200);
    icon.mockClear();
    for (let i = 0; i < 4; i++) ensurePageLoaded(page(`Unrelated ${i}`));
    expect(icon).not.toHaveBeenCalled();
    ensurePageLoaded(page("Property target", "icon:: AB"));
    expect(root.querySelectorAll(".page-ref-icon")).toHaveLength(200);
    setPageProperty("Property target", "icon", "CD");
    expect(root.querySelector(".page-ref-icon")?.textContent).toBe("CD");
    icon.mockClear();
    ensurePageLoaded(page("Another unrelated source"));
    expect(icon).not.toHaveBeenCalled();
  } finally { dispose(); }
});
