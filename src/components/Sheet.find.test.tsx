import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type Node } from "../document/model";
import { revealFindSheetBlock } from "../sheet/find";
import { cellSel } from "../sheet/selection";

beforeAll(initParser);
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

it.each(["grid", "table"])("Find reveals a %s match beyond its initial render window without selecting it", (kind) => {
  const rows = Array.from({ length: 222 }, (_, i) => `row-${i}`);
  const byId: Record<string, Node> = {};
  byId.owner = { id: "owner", raw: `Sheet\ntine.view:: ${kind}`, collapsed: false, parent: null, page: "Sheet", children: rows };
  for (const [index, id] of rows.entries()) {
    byId[id] = { id, raw: kind === "grid" ? "" : `target ${index}`, collapsed: false, parent: "owner", page: "Sheet", children: kind === "grid" ? [`cell-${index}`] : [] };
    if (kind === "grid") byId[`cell-${index}`] = { id: `cell-${index}`, raw: `target ${index}`, collapsed: false, parent: id, page: "Sheet", children: [] };
  }
  setDoc({ loaded: true, feed: ["Sheet"], pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots: ["owner"], format: "md", readOnly: false, guide: false }], byId });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="owner" />, host);
  try {
    const target = kind === "grid" ? "cell-221" : "row-221";
    expect(host.querySelector(`[data-block-id="${target}"]`)).toBeNull();
    const before = cellSel();
    revealFindSheetBlock(target, host);
    expect(host.querySelector(`[data-block-id="${target}"]`)).not.toBeNull();
    expect(cellSel()).toBe(before);
  } finally { dispose(); }
});
