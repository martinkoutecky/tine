import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, setCollapsed } from "../document";
import { type FeedPage, type Node as StoreNode, setDoc } from "../document/model";
import { BlockList } from "./BlockList";

const PAGE = "Fold page";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function seed(startCollapsed: boolean, children: number): void {
  const kids = Array.from({ length: children }, (_, i) => `kid-${i}`);
  const row = (id: string, raw: string, parent: string | null, ids: string[] = [], collapsed = false): StoreNode =>
    ({ id, raw, collapsed, parent, page: PAGE, children: ids });
  const page: FeedPage = { name: PAGE, kind: "page", title: PAGE, preBlock: null, roots: ["above", "fold"], format: "md", readOnly: false, guide: false };
  setDoc({
    byId: Object.fromEntries([
      row("above", "stays put", null),
      row("fold", "folded branch", null, kids, startCollapsed),
      ...kids.map(id => row(id, id, "fold")),
    ].map(n => [n.id, n])),
    pages: [page],
    feed: [PAGE],
    loaded: true,
  });
}

function rowsAboveAcrossFolds(startCollapsed: boolean, children: number): { same: boolean; shown: boolean } {
  seed(startCollapsed, children);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <BlockList ids={["above", "fold"]} />, host);
  try {
    const before = host.querySelector('[data-block-id="above"]');
    let same = before !== null;
    let state = startCollapsed;
    for (let i = 0; i < 3; i++) {
      state = !state;
      setCollapsed("fold", state);
      same &&= host.querySelector('[data-block-id="above"]') === before;
    }
    return { same, shown: host.querySelector('[data-block-id="kid-0"]') !== null };
  } finally {
    dispose();
  }
}

describe("folding never replaces the rows of a list (GH #795)", () => {
  it.each([
    ["starting expanded", false],
    ["starting collapsed", true],
  ])("keeps the row above a large branch, %s", (_name, startCollapsed) => {
    const { same, shown } = rowsAboveAcrossFolds(startCollapsed, 85);
    expect(same).toBe(true);
    expect(shown).toBe(startCollapsed);
  });

  it("keeps the row above a small branch", () => {
    expect(rowsAboveAcrossFolds(false, 5).same).toBe(true);
  });
});
