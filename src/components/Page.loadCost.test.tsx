import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { DEV, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { resetNearObserverForTests } from "../lazyObserve";
import { endEdit } from "../editorController";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import type { PageDto, PageRead } from "../types";
import { BlockList } from "./BlockList";
import { PageView } from "./Page";

beforeAll(initParser);
afterEach(() => { endEdit("blur"); vi.restoreAllMocks(); vi.unstubAllGlobals(); resetStore(); resetNearObserverForTests(); document.body.innerHTML = ""; resetTabsToJournals(); });

function dtoOf(name: string, n: number): PageDto {
  return { name, title: name, kind: "page", pre_block: null, format: "md",
    blocks: Array.from({ length: n }, (_, i) => ({ id: `${name}-${i}`, raw: "Plain leaf", children: [], collapsed: false })) };
}

/** Owners created while `view` mounts and shows `n` root rows of a fresh page. */
async function owners(n: number, view: (dto: PageDto) => () => JSX.Element): Promise<number> {
  const dto = dtoOf(`Cost ${n}`, n);
  loadSingle(dto);
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
  mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  let count = 0;
  const hooks = DEV!.hooks, previous = hooks.afterCreateOwner;
  hooks.afterCreateOwner = () => { count++; };
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(view(dto), host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".ls-block")).toHaveLength(n));
    await new Promise((r) => setTimeout(r, 0));
    return count;
  } finally {
    hooks.afterCreateOwner = previous;
    dispose(); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); resetStore(); resetNearObserverForTests();
  }
}

// Review 716a8eb3b (PC-1, REG-BIGLOAD-ROW-WORK on the Page path): a routed
// page's ordinary root costs what it cost before the opening row existed
// (163 owners, measured at develop 4f9865e35; a bare BlockList row is 157, the
// rest is the page's own surface). Only a row that can be the header shown as
// a block carries the opening-row slot. Both sizes fit in one outline window.
it("adds no per-row owner to a routed page's ordinary roots", async () => {
  const perRow = async (view: (dto: PageDto) => () => JSX.Element) => ((await owners(20, view)) - (await owners(10, view))) / 10;
  const list = await perRow((dto) => () => <BlockList ids={dto.blocks.map((b) => b.id)} />);
  const page = await perRow(() => () => <PageView />);
  expect(list).toBe(157);
  expect(page).toBeLessThanOrEqual(163);
});
