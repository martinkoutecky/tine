import { expect, it } from "vitest";
import { openPublishedPermalink, publishedPermalinkHash, resolvePublishedPermalink } from "./publishedPermalink";
import type { PublishedSnapshot } from "./publishedBackend";
import { createPaneRouter } from "./router";

function snapshot(): PublishedSnapshot {
  return {
    schema: 1, name: "Test", exported_at: "", home: "Cafe\u0301",
    pages: [{ name: "Cafe\u0301", kind: "page", path: "pages/cafe.md", title: "Café", pre_block: null,
      read_only: true, blocks: [] }],
    entries: [], backlinks: {}, block_ref_counts: {}, aliases: [["Re\u0301sume\u0301", "/Café/"]], icons: {}, queries: [],
  };
}

it.each([" /CAFÉ/ ", "/Résumé/"])("opens a permalink using canonical page/alias identity: %s", (page) => {
  const router = createPaneRouter();
  const result = openPublishedPermalink(snapshot(), publishedPermalinkHash({ kind: "page", page }), router);
  expect(result.status).toBe("opened");
  expect(router.route()).toMatchObject({ kind: "page", name: "Cafe\u0301" });
});

it.each(["md", "org"] as const)("block permalinks use authored %s IDs, ignoring literal examples and stale runtime IDs", format => {
  const s = snapshot(); s.pages[0].format = format;
  s.pages[0].blocks = [{ id: "literal", raw: format === "md" ? "```\nid:: wanted\n```" : "#+BEGIN_SRC\n:id: wanted\n#+END_SRC", collapsed: false, children: [] }];
  expect(resolvePublishedPermalink(s, { kind: "block", block: "wanted" })).toBeNull();
  s.pages[0].blocks.push({ id: "runtime", raw: format === "md" ? "Real\nid:: wanted" : "Real\n:PROPERTIES:\n:id: wanted\n:END:", collapsed: false, children: [] });
  expect(resolvePublishedPermalink(s, { kind: "block", block: "wanted" })?.page).toBe(s.pages[0]);
  expect(resolvePublishedPermalink(s, { kind: "block", block: "runtime" })).toBeNull();
  expect(resolvePublishedPermalink(s, { kind: "block", block: "literal" })?.page).toBe(s.pages[0]);
});
