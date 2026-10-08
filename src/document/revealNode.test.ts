import { expect, it } from "vitest";
import { node, revealNode, isDirty, resetStore, undo } from "./index";
import { setDoc } from "./model";

it("reveals a node in memory without dirtying its page or recording history", () => {
  resetStore();
  setDoc({
    byId: { root: { id: "root", raw: "parent", collapsed: true, parent: null, page: "Reveal", children: [] } },
    pages: [{ name: "Reveal", kind: "page", title: "Reveal", preBlock: null, roots: ["root"], format: "md", readOnly: false, guide: false }],
    feed: ["Reveal"], loaded: true,
  });
  expect(isDirty("Reveal")).toBe(false);
  revealNode("root");
  expect(node("root").collapsed).toBe(false);
  expect(isDirty("Reveal")).toBe(false);
  undo();
  expect(node("root").collapsed).toBe(false);
  resetStore();
});
