// Outline-kernel characterization (DESIGN §9): drive the window's edit
// functions over the shared verb fixtures that the Rust kernel is checked
// against (crates/tine-outline/tests/ops_fixtures.rs). Since M1 the edits run
// through the kernel adapter (document/kernel.ts), so this is the WASM leg of
// the both-ways check, plus the adapter's caret and store mapping. With
// TINE_RECORD_KERNEL_FIXTURES=1 the observed results are written back as
// `expect`; a difference from the Rust transcription is investigated, never
// resolved by editing `expect` alone.
import { readFileSync, writeFileSync } from "node:fs";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { BlockDto } from "../../types";

let nextKey = 0;
let logical = false;
let dirty: Record<string, string[]> = {};
let focus: { key: string; editor_offset: number | null } | null = null;
const KEEP = { start: -1, end: -1 };

vi.mock("../model", async (orig) => ({
  ...(await orig<typeof import("../model")>()),
  freshId: () => `n${nextKey++}`,
  keySource: () => ({ prefix: "n", next: nextKey }),
  keysUsed: (count: number) => { nextKey += count; },
}));
vi.mock("../../ui", async (orig) => ({ ...(await orig<typeof import("../../ui")>()), logicalOutdenting: () => logical }));
// `markDirty` is replaced through a proxy over the real module, loaded in
// beforeAll: an importOriginal factory hands blocks.ts the real module, since
// the real wiring's own imports reach blocks.ts while the factory runs.
let actualWiring: Record<string, unknown> | null = null;
const recordDirty = (name: string, kinds: string | readonly string[]) => {
  const list = (dirty[name] ??= []);
  for (const kind of typeof kinds === "string" ? [kinds] : kinds) if (!list.includes(kind)) list.push(kind);
};
vi.mock("../host/wiring", () => new Proxy({}, {
  get: (_target, key) => (key === "markDirty" ? recordDirty : key === "then" ? undefined : actualWiring?.[key as string]),
  has: () => true,
}));
vi.mock("../../editorController", async (orig) => ({
  ...(await orig<typeof import("../../editorController")>()),
  startEditing: (key: string, offset: unknown) => {
    focus = { key, editor_offset: offset === KEEP ? null : (offset as number) };
  },
}));

import { initParser } from "../../render/parse";
import { clearSeededFacets } from "../../render/facets";
import { BUILTIN_HIDE, rawOffsetToVisibleOffset } from "../../editor/properties";
import { doc, setDoc } from "../model";
import { nextVisible } from "../tree";
import { loadFeed, resetStore } from "../workingSet";
import { deleteBlock, indentBlock, insertEmptyChildBlock, mergeWithNext, outdentBlock, splitBlock } from "./blocks";
import { moveBlock, moveBlocksRelative, moveItem, moveSelectionItems } from "./moves";
import { clearSelection, deleteSelection, extendSelectionTo, indentSelection, outdentSelection, selectBlock, topSelected } from "./selection";

interface TreeNode { key: string; raw: string; children?: TreeNode[]; collapsed?: boolean }
interface FixturePage { name: string; format: "md" | "org"; roots: TreeNode[]; header_root?: string; read_only?: boolean; frozen?: boolean }
interface Expect { page?: FixturePage; kinds?: string[]; focus?: { key: string; editor_offset: number | null } | null; refused?: boolean; reason?: string }
interface Case { name: string; page: FixturePage; verb: Record<string, any>; expect?: Expect }

const FIXTURE = new URL("../../../crates/tine-outline/tests/fixtures/ops/m1.json", import.meta.url);
const fixture = JSON.parse(readFileSync(FIXTURE, "utf8")) as { _comment: string; cases: Case[] };
// "1" records every case; "missing" records only cases without `expect`.
const RECORD_MODE = process.env.TINE_RECORD_KERNEL_FIXTURES;
const RECORD = RECORD_MODE === "1" || RECORD_MODE === "missing";

/** The view collapse the fixtures assume: everything shown, except a split's
 * block with children whose `children_visible` is false. Mirrored by the Rust
 * runner. */
function toDto(node: TreeNode, collapsedKey: string | null): BlockDto {
  return { id: node.key, raw: node.raw, collapsed: node.key === collapsedKey && !!node.children?.length, children: (node.children ?? []).map((c) => toDto(c, collapsedKey)) };
}

function snapshot(name: string): FixturePage {
  const page = doc.pages.find((p) => p.name === name)!;
  const tree = (key: string): TreeNode => {
    const n = doc.byId[key];
    const out: TreeNode = { key, raw: n.raw };
    if (n.children.length) out.children = n.children.map(tree);
    if (n.collapsed) out.collapsed = true;
    return out;
  };
  return { name, format: page.format, roots: page.roots.map(tree) };
}

const utf16 = (raw: string, bytes: number) => new TextDecoder().decode(new TextEncoder().encode(raw).subarray(0, bytes)).length;

/** Select `keys` as the window does: a visible range whose top-level blocks
 *  are exactly `keys`. */
function selectKeys(c: Case, keys: string[]): void {
  selectBlock(keys[0]);
  extendSelectionTo(keys[keys.length - 1]);
  if (JSON.stringify(topSelected()) !== JSON.stringify(keys)) throw new Error(`${c.name}: the selection's top-level blocks are not ${keys}`);
}

async function run(c: Case): Promise<Expect> {
  const v = c.verb;
  const collapsedKey = v.verb === "split" && !v.children_visible ? v.key : null;
  loadFeed([{ name: c.page.name, kind: "page", title: c.page.name, pre_block: null, format: c.page.format, read_only: c.page.read_only ?? false, blocks: c.page.roots.map((r) => toDto(r, collapsedKey)) }]);
  clearSeededFacets();
  if (c.page.header_root) setDoc("byId", c.page.header_root, "originatedFromPageHeader", true);
  const before = JSON.stringify(snapshot(c.page.name));
  nextKey = 0;
  dirty = {};
  focus = null;
  logical = false;
  let depthRefusal = false;
  switch (v.verb) {
    case "split": {
      const raw = doc.byId[v.key].raw;
      const offset = rawOffsetToVisibleOffset(raw, utf16(raw, v.raw_offset), BUILTIN_HIDE, c.page.format);
      splitBlock(v.key, offset, v.force_child ?? false, v.keep_start_in_scope ?? false, null);
      break;
    }
    case "merge":
      if (nextVisible(v.survivor) !== v.absorbed) throw new Error(`${c.name}: the absorbed block must follow the survivor in visible order`);
      mergeWithNext(v.survivor, null, null);
      break;
    case "indent":
      if (v.keys.length > 1) {
        selectKeys(c, v.keys);
        indentSelection();
      } else {
        depthRefusal = indentBlock(v.keys[0], KEEP as never, null) === false;
      }
      break;
    case "outdent":
      logical = v.logical ?? false;
      if (v.keys.length > 1) {
        if (!logical) throw new Error(`${c.name}: a selection outdents logically`);
        selectKeys(c, v.keys);
        outdentSelection();
      } else {
        outdentBlock(v.keys[0], KEEP as never, null);
      }
      break;
    case "delete":
      if (v.keys.length > 1) {
        selectKeys(c, v.keys);
        deleteSelection();
      } else {
        deleteBlock(v.keys[0]);
      }
      break;
    case "move_up_down":
      if (v.within_siblings) {
        selectKeys(c, v.keys);
        await moveSelectionItems(v.up ? -1 : 1);
      } else {
        moveItem(v.keys[0], v.up ? -1 : 1);
      }
      break;
    case "move_to": {
      const to = v.to;
      if (to.at === "slot") {
        if (v.keys.length !== 1 || (v.inherit_from ?? null) !== to.parent) throw new Error(`${c.name}: a slot move is moveBlock's: one key, inheriting from the parent`);
        await moveBlock(v.keys[0], to.parent, to.index, to.page);
      } else {
        if (v.inherit_from !== to.key) throw new Error(`${c.name}: a relative move inherits from its target`);
        await moveBlocksRelative(v.keys, to.key, to.at === "last_child" ? "child" : to.at);
      }
      break;
    }
    case "insert_empty_child": {
      const made = insertEmptyChildBlock(v.parent, v.at);
      if (made) focus = { key: made, editor_offset: 0 };
      break;
    }
    default:
      throw new Error(`unknown verb ${v.verb}`);
  }
  const after = snapshot(c.page.name);
  if (JSON.stringify(after) === before && !Object.keys(dirty).length) {
    return { refused: true, ...(depthRefusal ? { reason: "depth_cap" } : {}) };
  }
  return { page: after, kinds: dirty[c.page.name] ?? [], focus };
}

beforeAll(async () => {
  actualWiring = await vi.importActual("../host/wiring");
  await initParser();
});
afterEach(() => {
  clearSelection();
  resetStore();
});

describe("outline kernel fixtures through the window's TS edits", () => {
  for (const c of fixture.cases) {
    it(c.name, async () => {
      if (RECORD_MODE === "missing" && c.expect) return;
      const got = await run(c);
      if (RECORD) {
        c.expect = { ...got, ...(c.expect?.reason && got.refused ? { reason: c.expect.reason } : {}) };
        return;
      }
      expect(c.expect, "fixture has no expectation; record it").toBeDefined();
      const { reason: _reason, ...want } = c.expect!;
      const { reason: _got, ...have } = got;
      expect(have).toEqual(want);
    });
  }
  if (RECORD) {
    it("writes the recorded expectations", () => {
      writeFileSync(FIXTURE, JSON.stringify(fixture, null, 1) + "\n");
    });
  }
});
