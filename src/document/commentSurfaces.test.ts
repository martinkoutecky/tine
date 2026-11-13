// H9TvA / D-20261113-CO: outward copies include marked comments by default.
import { afterEach, describe, expect, it } from "vitest";
import { loadFeed, resetStore, pageByName, ensureEmptyBlock, exportNodesFor, blockSubtreeMarkdown, buildClipboardPayload, nextVisible } from ".";
import { doc } from "./model";
import { threadRootOf } from "./commentThreads";
import { dtoSubtreeMarkdown } from "./edits/serialize";
import { exportOutline, DEFAULT_EXPORT_OPTIONS } from "../editor/exportText";
import { exportHtml } from "../editor/exportHtml";
import { exportOpml } from "../editor/exportOpml";
import { expandExportNodes } from "../editor/exportMarkup";
import type { ExportNode } from "../editor/exportText";
import type { BlockDto, Format } from "../types";
import { blockRegions } from "../render/parse";
import { collectInPageFindMatches } from "../inpageFind";
import flat from "../../tests/fixtures/comment-output/flat.json";

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
    expect(exportOutline(exportNodesFor([id]), { ...DEFAULT_EXPORT_OPTIONS, includeComments: false })).toContain("COMMENTMARKER");
  });

  it("marks comments by default, preserves source markers, and omits whole threads on request", () => {
    const { bodyId } = fixture(format);
    const nodes = exportNodesFor([bodyId]);
    expect(nodes[0].children[0].children[0].raw).toBe("REPLYMARKER");
    const options = { stripLinks: false, removeEmphasis: false, removeTags: false };
    for (const output of [
      exportOutline(nodes, DEFAULT_EXPORT_OPTIONS),
      exportHtml(nodes, options), exportOpml(nodes, options),
    ]) {
      expect(output).toContain("COMMENTMARKER");
      expect(output).toContain("REPLYMARKER");
      expect(output).toContain("Comment");
      expect(output).not.toContain("tine.quote");
    }
    for (const removeProperties of [false, true]) {
      expect(exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source", removeProperties })).toContain("tine.quote");
    }
    for (const output of [
      exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, includeComments: false }),
      exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source", includeComments: false }),
      exportHtml(nodes, { ...options, includeComments: false }),
      exportOpml(nodes, { ...options, includeComments: false }),
    ]) {
      expect(output).toContain("Body passage");
      expect(output).not.toContain("COMMENTMARKER");
      expect(output).not.toContain("REPLYMARKER");
    }
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

  it("omits a selected comment or reply without treating its host as a thread", () => {
    const { commentId, replyId, bodyId } = fixture(format);
    for (const id of [commentId, replyId]) {
      expect(exportOutline(exportNodesFor([id]), { ...DEFAULT_EXPORT_OPTIONS, includeComments: false })).toBe("");
    }
    expect(exportOutline(exportNodesFor([bodyId]), { ...DEFAULT_EXPORT_OPTIONS, includeComments: false })).toBe("- Body passage");
  });

  it("includes comment and reply text in the in-page Find count", () => {
    const { root } = fixture(format);
    expect(collectInPageFindMatches([root], "COMMENTMARKER", format)).toHaveLength(1);
    expect(collectInPageFindMatches([root], "REPLYMARKER", format)).toHaveLength(1);
    expect(collectInPageFindMatches([root], "tine.quote", format)).toHaveLength(0);
  });
});

it("freezes the comment marking for each flat format", () => {
  const { bodyId } = fixture("md");
  const nodes = exportNodesFor([bodyId]);
  const options = { stripLinks: false, removeEmphasis: false, removeTags: false };
  expect({
    text: exportOutline(nodes, DEFAULT_EXPORT_OPTIONS),
    source: exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source" }),
    html: exportHtml(nodes, options),
    opml: exportOpml(nodes, options),
  }).toEqual(flat);
});

it("attributes flat annotations and retains their author/date in source with Remove properties", () => {
  const nodes = [{ raw: "Body", children: [{ raw: "Note\ntine.quote:: Body\ntine.author:: Ada\ntine.date:: 2026-11-13T00:00Z", children: [] }] }];
  const options = { stripLinks: false, removeEmphasis: false, removeTags: false };
  for (const output of [exportOutline(nodes, DEFAULT_EXPORT_OPTIONS), exportHtml(nodes, options), exportOpml(nodes, options)]) {
    expect(output).toContain("Comment · Ada · 2026-11-13T00:00Z");
  }
  const source = exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source", removeProperties: true });
  for (const property of ["tine.quote:: Body", "tine.author:: Ada", "tine.date:: 2026-11-13T00:00Z"]) expect(source).toContain(property);
});

it.each(["md", "org"] as const)("preserves a detached embedded reply as a comment in %s source", format => {
  const reply: ExportNode = { raw: "REPLYMARKER", format, hasParent: false,
    comment: { author: null, date: null }, children: [] };
  const nodes: ExportNode[] = [{ raw: "Body", format, children: [
    { raw: "{{embed ((6a1b2c3d-0000-4000-8000-000000000010))}}", format, children: [] },
  ] }];
  const resolveEmbed = () => [reply];
  const options = { ...DEFAULT_EXPORT_OPTIONS, resolveEmbed };
  for (const output of [exportOutline(expandExportNodes(nodes, options), options), exportHtml(nodes, options), exportOpml(nodes, options)]) {
    expect(output).toContain("REPLYMARKER");
    expect(output).toContain("Comment");
  }
  for (const output of [
    exportOutline(expandExportNodes(nodes, options), { ...options, includeComments: false }),
    exportOutline(nodes, { ...options, content: "source", includeComments: false }),
    exportHtml(nodes, { ...options, includeComments: false }),
    exportOpml(nodes, { ...options, includeComments: false }),
  ]) {
    expect(output).toContain("Body");
    expect(output).not.toContain("REPLYMARKER");
  }
  for (const removeProperties of [false, true]) {
    const source = exportOutline(nodes, { ...options, content: "source", removeProperties });
    expect(source).toContain("REPLYMARKER");
    expect(source).toContain("tine.quote");
  }
  // A source-only copy must retain the meaning after transient DTO bits are gone.
  const materialized = expandExportNodes(nodes, options)[0].children[0];
  const copied: ExportNode[] = [{ raw: "Body", format, children: [
    { raw: materialized.raw, format, children: [] },
  ] }];
  expect(exportOutline(copied, DEFAULT_EXPORT_OPTIONS)).toContain("[Comment] REPLYMARKER");
  expect(exportOutline(copied, { ...DEFAULT_EXPORT_OPTIONS, includeComments: false })).toBe("- Body");
  expect(reply.raw).toBe("REPLYMARKER");
});

it.each(["md", "org"] as const)("keeps expanded embeds inside the original comment in %s source-only copies", (format) => {
  const metadata = format === "md" ? "\ntine.quote:: Body\ntine.author:: Ada" : "\n:PROPERTIES:\n:tine.quote: Body\n:tine.author: Ada\n:END:";
  const targetAuthor = format === "md" ? "\ntine.author:: Bob" : "\n:PROPERTIES:\n:tine.author: Bob\n:END:";
  const nodes: ExportNode[] = [{ raw: "Body", format, children: [{
    raw: `{{embed [[Target]]}}${metadata}`, format,
    children: [{ raw: "Reply", children: [] }],
  }] }];
  const options = { stripLinks: false, removeEmphasis: false, removeTags: false,
    resolveEmbed: () => [{ raw: `Embedded${targetAuthor}`, format, children: [{ raw: "Nested target child", children: [] }] },
      { raw: "Second target root", children: [] }],
  };
  const html = exportHtml(nodes, options);
  expect(html.match(/<aside class="comment">/g)).toHaveLength(5);
  expect(html).toContain("Comment · Ada");
  expect(html).toContain("Comment · Bob");
  const opml = exportOpml(nodes, options);
  expect(opml.match(/text="\[Comment/g)).toHaveLength(5);
  const sourceOnly = (forest: ExportNode[]): ExportNode[] => forest.map(node => ({ raw: node.raw, format: node.format, children: sourceOnly(node.children) }));
  const copied = sourceOnly(expandExportNodes(nodes, options));
  const source = exportOutline(copied, { ...DEFAULT_EXPORT_OPTIONS, content: "source" });
  for (const text of ["tine.quote", "Ada", "Bob", "Embedded", "Nested target child", "Second target root", "Reply"]) expect(source).toContain(text);
  expect(exportOutline(copied, { ...DEFAULT_EXPORT_OPTIONS, content: "source", includeComments: false })).toBe("- Body");
  for (const output of [exportHtml(nodes, { ...options, includeComments: false }), exportOpml(nodes, { ...options, includeComments: false })]) {
    expect(output).toContain("Body");
    expect(output).not.toMatch(/Embedded|Nested target child|Second target root|Reply/);
  }
});

it("seeds a body bullet after a Markdown properties-only root even when that root holds a thread", () => {
  const { bodyId, commentId } = fixture("md", "note:: value");
  const seeded = ensureEmptyBlock("Comment surfaces", { afterProperties: true });
  expect(seeded).toBeTruthy();
  expect(pageByName("Comment surfaces")!.roots).toEqual([bodyId, seeded]);
  expect(doc.byId[bodyId].children).toEqual([commentId]);
  expect(threadRootOf(commentId)).toBe(commentId);
});
