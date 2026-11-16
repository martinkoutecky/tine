import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { adoptResults, advanceResultsEpoch, captureResultsTicket } from "../queryResultsEpoch";
import { LiveRefGroup } from "./LiveRefGroup";
import type { BlockDto } from "../types";

// GH #851: a rerun replaces the result rows with new objects; the local expand/collapse choices belong to the
// group, not to the row objects, so they survive it.
beforeAll(initParser);
afterEach(() => { resetStore(); document.body.replaceChildren(); });
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
async function load() {
  setDoc({ byId: {
    task: { id: "task", raw: "TODO Selected", collapsed: false, parent: null, page: "Source", children: ["kid"] },
    kid: { id: "kid", raw: "Kid", collapsed: false, parent: "task", page: "Source", children: ["grand"] },
    grand: { id: "grand", raw: "Grand", collapsed: false, parent: "kid", page: "Source", children: [] },
    today: { id: "today", raw: "Journal entry", collapsed: false, parent: null, page: "Today", children: [] },
  }, pages: [
    { id: "pages/Source.md", name: "Source", kind: "page", title: "Source", preBlock: null, roots: ["task"], format: "md", readOnly: false, guide: false },
    { id: "journals/Today.md", name: "Today", kind: "journal", title: "Today", preBlock: null, roots: ["today"], format: "md", readOnly: false, guide: false },
  ], feed: ["Today"], loaded: true });
  await Promise.resolve();
}
const dto = (): BlockDto => ({ id: "task", raw: "TODO Selected", marker: "TODO", collapsed: false, children: [] });
const adopted = (): BlockDto => { const hit = dto(); expect(adoptResults(hit, captureResultsTicket())).toBe(hit); return hit; };

it("a rerun that replaces the result DTOs keeps an expanded row expanded", async () => {
  await load();
  const root = document.createElement("div"); document.body.append(root);
  const [blocks, setBlocks] = createSignal<BlockDto[]>([adopted()]);
  const dispose = render(() => <LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks()} surface="query" eager />, root);
  try {
    await tick(); await tick();
    const row = (id: string) => root.querySelector(`[data-block-id="${id}"]`);
    expect(row("kid")).not.toBeNull();
    expect(row("kid")!.classList.contains("collapsed")).toBe(true); // below the hit starts folded
    row("kid")!.querySelector<HTMLElement>(".collapse-toggle")!.click();
    await tick();
    expect(row("grand")).not.toBeNull();
    // the edit's structural step moves the epoch; the rerun then lands new DTOs
    advanceResultsEpoch();
    await tick();
    setBlocks([adopted()]);
    for (let i = 0; i < 5; i++) { await tick(); }
    expect(row("grand")).not.toBeNull();
    expect(row("kid")!.classList.contains("collapsed")).toBe(false);
  } finally { dispose(); }
});
