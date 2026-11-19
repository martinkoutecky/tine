import { afterEach, beforeAll, expect, it, vi } from "vitest";

// Count identity parses on the loaded export path (GH #793 review 7f0512102).
const parses = vi.hoisted(() => ({ n: 0 }));
vi.mock("../blockIdentity", async original => {
  const actual = await original<typeof import("../blockIdentity")>();
  return { ...actual, existingBlockId: (...args: Parameters<typeof actual.existingBlockId>) => { parses.n++; return actual.existingBlockId(...args); } };
});

import { closeExportModal, exportModal } from "../ui";
import { openReferenceResultsCopy } from "./queryResultsCopy";
import type { BlockDto, RefGroup } from "../types";
import { initParser } from "../render/parse";
import { ensurePageLoaded, resetStore } from "../document";
import { graphOwner } from "../owned";

beforeAll(initParser);
afterEach(() => { closeExportModal(); resetStore(); });

const block = (id: string, raw: string): BlockDto => ({ id, raw, children: [], collapsed: false });

it.each([false, true])("exports one leaf of a loaded 2,000-block page without walking the page (authored id: %s)", async authored => {
  const leafRaw = authored ? "Leaf [[Target]]\nid:: 6f9619ff-8b86-4011-b42d-00c04fc964ff" : "Leaf [[Target]]";
  const blocks = Array.from({ length: 2000 }, (_, i) => block(`b${i}`, `Block ${i}`));
  blocks[1500] = block("b1500", leafRaw);
  ensurePageLoaded({ id: "pages/Big.md", name: "Big", title: "Big", kind: "page", format: "md", pre_block: null, blocks });
  const groups: RefGroup[] = [{ page: "Big", kind: "page", path: "pages/Big.md",
    blocks: [{ ...block("b1500", leafRaw), ...(authored ? { properties: [["id", "6f9619ff-8b86-4011-b42d-00c04fc964ff"]] as [string, string][] } : {}) }] }];
  parses.n = 0;
  expect(await openReferenceResultsCopy(groups, graphOwner())).toBe(true);
  expect(parses.n).toBeLessThan(50);
  const modal = exportModal();
  expect(JSON.stringify(modal)).toContain("Leaf [[Target]]");
  expect(JSON.stringify(modal)).not.toContain("Block 7");
});
