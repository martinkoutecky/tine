import { vi } from "vitest";
import { backend } from "../backend";
import { applyGraphChangesBulk, ensurePageLoaded, pageByName, node } from "../document";
import { pageIdentityKey } from "../pageIdentity";
import type { ConflictObject, PageRead } from "../types";

/** A loaded resolution target plus the native publication delivered by its
 * guarded write. Tests drive the real resolver and observe its live content. */
export function resolutionPublication(conflict: ConflictObject) {
  const before: PageRead = { name: conflict.page_name, title: conflict.page_name, kind: conflict.kind, id: conflict.page_path,
    rev: "before", pre_block: null, blocks: [{ id: "resolution-target", raw: "before resolution", collapsed: false, children: [] }] };
  let disk = before;
  const refusal = ensurePageLoaded(before);
  if (refusal) throw new Error("The synthetic resolution target was not admitted.");
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === disk.id ? disk : null);
  return {
    async publish() {
      disk = { ...before, rev: "resolved", blocks: [{ ...before.blocks[0], raw: "resolved live content" }] };
      await applyGraphChangesBulk({ changes: [], answers: { rev: "1", inventoryChanged: false, blockRefCounts: {},
        summary: { rev: "1", pages: [pageIdentityKey(before.name)], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
    },
    shown: () => pageByName(before.name)?.roots.map(id => node(id).raw),
  };
}
