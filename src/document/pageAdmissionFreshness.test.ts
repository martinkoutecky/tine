import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import type { PageDto, PageKind } from "../types";
import { doc, pageByName } from "./model";
import { ensurePageLoaded, loadFeed, loadRoutedPage, pinPageWhileDrafting, resetStore } from "./index";
import { baseRevFor } from "./host/wiring";
import { setRaw } from "./edits/blocks";

beforeAll(() => initParser());
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

const page = (kind: PageKind, merged = false): PageDto & { id: string } => ({
  name: "Oct 26th, 2026", title: "Oct 26th, 2026", kind,
  id: `${kind === "journal" ? "journals" : "pages"}/2026-10-26.md`,
  rev: merged ? "merged" : "before", pre_block: null,
  blocks: (merged ? ["only in iso", "only in underscore"] : ["only in iso"]).map((raw, i) => ({
    id: `sentinel-${i}`, raw, collapsed: false, children: [],
  })),
});
const shown = () => pageByName("Oct 26th, 2026")!.roots.map(id => doc.byId[id].raw);

it("keeps unchanged-revision nodes without revisiting the DTO's blocks", () => {
  loadFeed([page("journal")]);
  const original = doc.byId[pageByName("Oct 26th, 2026")!.roots[0]];
  const dto = page("journal");
  const blocks = dto.blocks;
  let bodyReads = 0;
  Object.defineProperty(dto, "blocks", { get() { bodyReads++; return blocks; } });
  expect(loadRoutedPage(dto)).toBeNull();
  expect(ensurePageLoaded(dto)).toBeNull();
  expect(bodyReads).toBe(0);
  expect(doc.byId[pageByName("Oct 26th, 2026")!.roots[0]]).toBe(original);
});

for (const kind of ["journal", "page"] as const) it.each([
  ["routed", loadRoutedPage], ["satellite", ensurePageLoaded],
] as const)(`a fresh ${kind} read displays merged blocks through %s admission`, (_surface, admit) => {
  const read = vi.spyOn(backend(), "getPage");
  const readPath = vi.spyOn(backend(), "getPageByPath");
  loadFeed([page(kind)]);
  expect(admit(page(kind, true))).toBeNull();
  expect(shown()).toEqual(["only in iso", "only in underscore"]);
  expect(baseRevFor("Oct 26th, 2026")).toBe("merged");
  expect(read).not.toHaveBeenCalled();
  expect(readPath).not.toHaveBeenCalled();
});

it("a fresh read preserves an edit that has not reached the host", () => {
  loadFeed([page("journal")]);
  setRaw("sentinel-0", "unsent input", { timetracking: false });
  expect(loadRoutedPage(page("journal", true))).toBeNull();
  expect(shown()).toEqual(["unsent input"]);
  expect(baseRevFor("Oct 26th, 2026")).toBe("before");
});

it("a fresh read preserves a component's unsent draft and replays when its hold releases", async () => {
  loadRoutedPage(page("journal"));
  const read = vi.spyOn(backend(), "getPage").mockResolvedValue(page("journal", true));
  const release = pinPageWhileDrafting(() => "Oct 26th, 2026");
  try {
    expect(loadRoutedPage(page("journal", true))).toBeNull();
    expect(shown()).toEqual(["only in iso"]);
    expect(baseRevFor("Oct 26th, 2026")).toBe("before");
  } finally { release(); }
  await vi.waitFor(() => expect(shown()).toEqual(["only in iso", "only in underscore"]));
  expect(read).toHaveBeenCalledOnce();
});
