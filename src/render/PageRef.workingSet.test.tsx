import { afterEach, expect, it, vi } from "vitest";
import { batch, For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, resetStore, setPageProperty } from "../document";
import * as icons from "../pageIconBatch";
import { PageRef } from "./inline";

afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

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
