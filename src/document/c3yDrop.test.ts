// og c3y Y5: a relative drop (drag of a selection, "before"/"after"/"child")
// that the outline ceiling refuses must tell the user, through the same toast
// the single-block move uses ("Outline is too deep to move"), not vanish.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { isDirty, loadFeed, moveBlocksRelative, resetStore } from "./index";
import { doc } from "./model";
import { setToasts, toasts } from "../toasts";
import { OUTLINE_MAX_DEPTH } from "../editor/outline";
import type { BlockDto, PageDto } from "../types";

const uuid = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const page = (name: string, blocks: BlockDto[]): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev: `r1-${name}`, blocks, format: "md",
});
function chain(levels: number, base: number): BlockDto {
  let node = block(uuid(base + levels), `level ${levels}`);
  for (let level = levels - 1; level >= 1; level--) node = block(uuid(base + level), `level ${level}`, [node]);
  return node;
}
const tooDeep = () => toasts().filter((t) => /Outline is too deep to move/.test(t.message));

beforeAll(() => initParser());
beforeEach(() => {
  resetStore();
  setToasts([]);
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `saved-${i}`) }));
});
afterEach(() => {
  resetStore();
  setToasts([]);
  vi.restoreAllMocks();
});

describe("og c3y Y5", () => {
  it("a child drop refused by the outline ceiling shows the too-deep toast and changes nothing", async () => {
    loadFeed([page("A", [chain(OUTLINE_MAX_DEPTH, 1000)]), page("B", [block(uuid(20), "target")])]);
    expect(await moveBlocksRelative([uuid(1001)], uuid(20), "child")).toBe(false);
    expect(tooDeep()).toHaveLength(1);
    expect(doc.byId[uuid(1001)].page).toBe("A");
    expect(isDirty("A") || isDirty("B")).toBe(false);
  });

  it("a drop that fits shows no refusal", async () => {
    loadFeed([page("A", [chain(OUTLINE_MAX_DEPTH, 1000)]), page("B", [block(uuid(20), "target")])]);
    expect(await moveBlocksRelative([uuid(1001)], uuid(20), "after")).toBe(true);
    expect(tooDeep()).toHaveLength(0);
  });
});
