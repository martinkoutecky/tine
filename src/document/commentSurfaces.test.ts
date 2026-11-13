// H9TvA inventory evidence. Export inclusion is awaiting a product decision;
// these tests describe the current outputs, not a new comment export policy.
import { afterEach, describe, expect, it } from "vitest";
import { loadFeed, resetStore, pageByName, ensureEmptyBlock, exportNodesFor, blockSubtreeMarkdown, buildClipboardPayload, nextVisible } from ".";
import { doc } from "./model";
import { threadRootOf } from "./commentThreads";
import { dtoSubtreeMarkdown } from "./edits/serialize";
import { exportOutline, DEFAULT_EXPORT_OPTIONS } from "../editor/exportText";
import { exportHtml } from "../editor/exportHtml";
import { exportOpml } from "../editor/exportOpml";
import type { BlockDto, Format } from "../types";
import { blockRegions } from "../render/parse";
import { collectInPageFindMatches } from "../inpageFind";

afterEach(resetStore);

function fixture(format: Format, body = "Body passage") {
  const quote = format === "md" ? "\ntine.quote:: Body passage" : "\n:PROPERTIES:\n:tine.quote: Body passage\n:END:";
  const root: BlockDto = { id: "body", raw: body, collapsed: false, children: [
    { id: "comment", raw: `COMMENTMARKER${quote}`, collapsed: false, children: [
      { id: "reply", raw: "REPLYMARKER", collapsed: false, children: [] },
    ] },
  ] };
  root.children[0].properties = blockRegions(root.children[0].raw, format).properties.filter(p => p.primary).map(p => [p.key, p.value]);
  loadFeed([{ name: "Comment surfaces", title: "Comment surfaces", kind: "page", format, pre_block: null, blocks: [root] }]);
  const bodyId = pageByName("Comment surfaces")!.roots[0];
  const commentId = doc.byId[bodyId].children[0];
  return { root, bodyId, commentId, replyId: doc.byId[commentId].children[0] };
}

describe.each(["md", "org"] as const)("comment surface inventory (%s)", format => {
  it("keeps comments and replies out of body navigation, but preserves their host", () => {
    const { bodyId, commentId, replyId } = fixture(format, "");
    expect(threadRootOf(bodyId)).toBeNull();
    expect(threadRootOf(commentId)).toBe(commentId);
    expect(threadRootOf(replyId)).toBe(commentId);
    expect(nextVisible(bodyId)).toBeNull();
    expect(ensureEmptyBlock("Comment surfaces", { afterProperties: true })).toBeNull();
    expect(pageByName("Comment surfaces")!.roots).toEqual([bodyId]);
  });

  it("treats a root quote property as body content, not a page-level thread", () => {
    // The same parser-recognized properties have comment meaning only below a block.
    const raw = format === "md" ? "COMMENTMARKER\ntine.quote:: x" : "COMMENTMARKER\n:PROPERTIES:\n:tine.quote: x\n:END:";
    loadFeed([{ name: "Root quote", title: "Root quote", kind: "page", format, pre_block: null,
      blocks: [{ id: "root", raw, properties: blockRegions(raw, format).properties.filter(p => p.primary).map(p => [p.key, p.value]), collapsed: false, children: [] }] }]);
    const id = pageByName("Root quote")!.roots[0];
    expect(threadRootOf(id)).toBeNull();
    expect(nextVisible(id)).toBeNull();
    expect(ensureEmptyBlock("Root quote")).toBeNull();
  });

  it("retains unlabelled comment children in text, source, HTML and OPML exports", () => {
    const { bodyId } = fixture(format);
    const nodes = exportNodesFor([bodyId]);
    expect(nodes[0].children[0].children[0].raw).toBe("REPLYMARKER");
    const options = { stripLinks: false, removeEmphasis: false, removeTags: false };
    for (const output of [
      exportOutline(nodes, DEFAULT_EXPORT_OPTIONS),
      exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source", removeProperties: true }),
      exportHtml(nodes, options), exportOpml(nodes, options),
    ]) {
      expect(output).toContain("COMMENTMARKER");
      expect(output).toContain("REPLYMARKER");
      expect(output).not.toContain("tine.quote");
    }
    expect(exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source" })).not.toContain("tine.quote");
  });

  it("keeps comment metadata and descendants in live/fetched Markdown copy and private clipboard", () => {
    const { bodyId, root } = fixture(format);
    for (const output of [blockSubtreeMarkdown(bodyId), dtoSubtreeMarkdown(root)]) {
      expect(output).toContain("COMMENTMARKER");
      expect(output).toContain("tine.quote");
      expect(output).toContain("REPLYMARKER");
    }
    expect(buildClipboardPayload([bodyId])!.blocks[0].children[0].raw).toContain("tine.quote");
  });

  it("includes comment and reply text in the in-page Find count", () => {
    const { root } = fixture(format);
    expect(collectInPageFindMatches([root], "COMMENTMARKER", format)).toHaveLength(1);
    expect(collectInPageFindMatches([root], "REPLYMARKER", format)).toHaveLength(1);
    expect(collectInPageFindMatches([root], "tine.quote", format)).toHaveLength(0);
  });
});

it("seeds a body bullet after a Markdown properties-only root even when that root holds a thread", () => {
  const { bodyId, commentId } = fixture("md", "note:: value");
  const seeded = ensureEmptyBlock("Comment surfaces", { afterProperties: true });
  expect(seeded).toBeTruthy();
  expect(pageByName("Comment surfaces")!.roots).toEqual([bodyId, seeded]);
  expect(doc.byId[bodyId].children).toEqual([commentId]);
  expect(threadRootOf(commentId)).toBe(commentId);
});
