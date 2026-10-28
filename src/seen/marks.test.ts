import { describe, expect, it } from "vitest";
import type { SeenPageChanges } from "../backendTypes";
import { changeCount, emptyMarks, marksFromChanges, nodeAtPath, SEEN_PAGE_ROOT, type SnapshotNode } from "./marks";

// A (A1, A2 (A2a)), B
const leaf = (id: string, raw: string, children: SnapshotNode[] = []): SnapshotNode => ({ id, raw, children });
const roots: SnapshotNode[] = [
  leaf("a", "A", [leaf("a1", "A1"), leaf("a2", "A2", [leaf("a2a", "A2a")])]),
  leaf("b", "B"),
];
const reply = (over: Partial<SeenPageChanges>): SeenPageChanges => ({
  available: true, reason: null, head_commit: "h", baseline_commit: "b", current_text_sha256: "x",
  identity_known: true, unattributed: false, blocks: [], removed: [], ...over,
});

describe("mapping a backend reply onto the rendered tree (ADR 0079)", () => {
  it("resolves zero-based structural paths [root, child, ...]", () => {
    expect(nodeAtPath(roots, [0, 1, 0])?.id).toBe("a2a");
    expect(nodeAtPath(roots, [1])?.id).toBe("b");
    expect(nodeAtPath(roots, [2])).toBeNull();
    expect(nodeAtPath(roots, [0, 5])).toBeNull();
  });

  it("marks the block at each block_path with its raw text at snapshot time", () => {
    const marks = marksFromChanges(reply({ blocks: [{ block_path: [0, 1], persisted_id: null }, { block_path: [1], persisted_id: "u" }] }), roots);
    expect(marks.blocks).toEqual({ a2: "A2", b: "B" });
    expect(changeCount(marks, (id) => ({ a2: "A2", b: "B" } as Record<string, string>)[id])).toBe(2);
  });

  it("ignores a path the snapshot does not have", () => {
    expect(marksFromChanges(reply({ blocks: [{ block_path: [9, 9], persisted_id: null }] }), roots).blocks).toEqual({});
  });

  it("a block edited after the snapshot loses its mark and its count", () => {
    const marks = marksFromChanges(reply({ blocks: [{ block_path: [0], persisted_id: null }, { block_path: [1], persisted_id: null }] }), roots);
    expect(changeCount(marks, (id) => (id === "a" ? "A edited" : "B"))).toBe(1);
  });

  it("places a removal after its previous sibling, else first in its parent, else first on the page", () => {
    const marks = marksFromChanges(reply({
      removed: [
        { text: "gone1", baseline_block_path: [0, 1], parent_path: [0], previous_sibling_path: [0, 0] },
        { text: "gone2", baseline_block_path: [0, 0], parent_path: [0], previous_sibling_path: null },
        { text: "gone3", baseline_block_path: [0], parent_path: null, previous_sibling_path: null },
        { text: "gone4", baseline_block_path: [1], parent_path: null, previous_sibling_path: [0] },
      ],
    }), roots);
    expect(marks.after.a1).toEqual([{ text: "gone1" }]);
    expect(marks.first.a).toEqual([{ text: "gone2" }]);
    expect(marks.first[SEEN_PAGE_ROOT]).toEqual([{ text: "gone3" }]);
    expect(marks.after.a).toEqual([{ text: "gone4" }]);
    expect(marks.removedCount).toBe(4);
    expect(changeCount(marks, () => undefined)).toBe(4);
  });

  it("never drops a removal whose anchors no longer resolve: it falls back to the page start", () => {
    const marks = marksFromChanges(reply({
      removed: [{ text: "orphan", baseline_block_path: [7], parent_path: [7], previous_sibling_path: [7, 0] }],
    }), roots);
    expect(marks.first[SEEN_PAGE_ROOT]).toEqual([{ text: "orphan" }]);
  });

  it("keeps several removals after one sibling in reply order", () => {
    const marks = marksFromChanges(reply({
      removed: [
        { text: "r1", baseline_block_path: [1], parent_path: null, previous_sibling_path: [0] },
        { text: "r2", baseline_block_path: [2], parent_path: null, previous_sibling_path: [0] },
      ],
    }), roots);
    expect(marks.after.a.map((m) => m.text)).toEqual(["r1", "r2"]);
  });

  it("carries the identity and attribution notes", () => {
    const marks = marksFromChanges(reply({ identity_known: false, unattributed: true }), roots);
    expect(marks.identityKnown).toBe(false);
    expect(marks.unattributed).toBe(true);
  });

  it("an unavailable reply shows nothing", () => {
    expect(marksFromChanges(reply({ available: false, reason: "not a repository", blocks: [{ block_path: [0], persisted_id: null }] }), roots)).toEqual(emptyMarks());
  });
});
