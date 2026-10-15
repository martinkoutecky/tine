import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { insertEmptyChildBlock, resetStore } from "../document";
import { reloadPage } from "../document/workingSet";
import { doc, setDoc } from "../document/model";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BlockDto } from "../types";

// UI-QUERY-RESULT-EPOCH-FLICKER: adding a block on today's journal made the
// Scheduled & Deadline results drop to read-only stand-ins (children, dates and
// breadcrumbs gone) until the query reran about a second later.
beforeAll(initParser);
afterEach(() => { resetStore(); document.body.replaceChildren(); });

async function load() {
  setDoc({ byId: {
    task: { id: "task", raw: "TODO Selected", collapsed: false, parent: null, page: "Source", children: ["child"] },
    child: { id: "child", raw: "Live child", collapsed: false, parent: "task", page: "Source", children: [] },
    today: { id: "today", raw: "Journal entry", collapsed: false, parent: null, page: "Today", children: [] },
  }, pages: [
    { id: "pages/Source.md", name: "Source", kind: "page", title: "Source", preBlock: null, roots: ["task"], format: "md", readOnly: false, guide: false },
    { id: "journals/Today.md", name: "Today", kind: "journal", title: "Today", preBlock: null, roots: ["today"], format: "md", readOnly: false, guide: false },
  ], feed: ["Today"], loaded: true });
  await Promise.resolve();
}
async function mountResult() {
  // The result DTO carries no children: only the live block shows "Live child".
  const hit: BlockDto = { id: "task", raw: "TODO Selected", marker: "TODO", collapsed: false, children: [] };
  expect(adoptResults(hit, captureResultsTicket())).toBe(hit);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={[hit]} surface="query" eager />, root);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(root.textContent).toContain("Live child");
  return { root, dispose };
}

it("keeps a mounted result live through a structural edit on another page", async () => {
  await load();
  const { root, dispose } = await mountResult();
  try {
    insertEmptyChildBlock("today", 0);
    expect(root.textContent).toContain("Live child");
    await Promise.resolve();
    expect(root.textContent).toContain("Live child");
  } finally { dispose(); }
});

it("keeps a typed-into result live when a structural edit follows", async () => {
  await load();
  const { root, dispose } = await mountResult();
  try {
    setDoc("byId", "task", "raw", "TODO Selected and edited");
    insertEmptyChildBlock("today", 0);
    await Promise.resolve();
    expect(root.textContent).toContain("Live child");
  } finally { dispose(); }
});

it("retires a mounted result when its page is replaced, even with equal text and parent", async () => {
  await load();
  const { root, dispose } = await mountResult();
  try {
    // A watcher or conflict reload installs backend content over the loaded
    // page: the same runtime id now names a block with another subtree.
    reloadPage({ id: "pages/Source.md", name: "Source", title: "Source", kind: "page", pre_block: null, rev: "r2",
      blocks: [{ id: "task", raw: "TODO Selected", marker: "TODO", collapsed: false,
        children: [{ id: "child", raw: "Replacement child", collapsed: false, children: [] }] }] });
    await Promise.resolve();
    expect(doc.byId.child.raw).toBe("Replacement child");
    expect(root.textContent).not.toContain("Replacement child");
  } finally { dispose(); }
});
