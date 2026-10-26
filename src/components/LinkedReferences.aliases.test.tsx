import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { readPageProperty, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { resetReferenceSectionState } from "../referenceSectionState";
import type { BacklinkFilterContext, RefGroup } from "../types";

vi.mock("./LiveRefGroup", () => ({ LiveRefGroup: (p: { blocks: { id: string }[] }) =>
  <div class="test-ref-group">{p.blocks.map(b => b.id).join(",")}</div> }));
beforeAll(() => initParser());
afterEach(() => { document.body.innerHTML = ""; localStorage.clear(); resetStore(); resetPageIndex(); resetReferenceSectionState(); vi.restoreAllMocks(); });

async function setup(pre: string | null = null, overlap = false, unlinked = false, format: "md" | "org" = "md") {
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
    { name: "Germany", key: "germany", is_journal: false, day: null, target: { kind: "existing", id: "pages/Germany.md", others: [] } },
    ...["DE", "Deutschland"].map(name => ({ name, key: name.toLowerCase(), is_journal: false, day: null, target: { kind: "alias" as const, owners: ["pages/Germany.md"] } })),
  ] });
  await refreshPageIndex();
  loadSingle({ name: "Target", kind: "page", title: "Target", pre_block: pre, format, blocks: [] });
  const names = [...Array(8).fill("Germany"), ...Array(3).fill("Deutschland"), ...Array(15).fill("DE"), "Zebra"];
  const groups: RefGroup[] = [{ page: "Source", kind: "page", blocks: names.map((name, i) => ({ id: `r${i}`, raw: `[[Target]] [[${name}]]`, tags: [name], children: [], collapsed: false })) }];
  const context: BacklinkFilterContext = { entries: names.map((name, i) => ({ page: "Source", kind: "page", block_id: `r${i}`, text: name, facets: overlap && i === 0 ? ["Germany", "DE", "Deutschland"] : [name] })) };
  vi.spyOn(backend(), unlinked ? "getUnlinkedRefs" : "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue(context);
  const root = document.createElement("div"); document.body.append(root);
  const mount = () => render(() => unlinked ? <UnlinkedReferences name="Target" /> : <LinkedReferences name="Target" />, root);
  let dispose = mount();
  const open = async () => {
    await vi.waitFor(() => expect(root.querySelector(".reference-filter-toggle")).not.toBeNull());
    if (!root.querySelector(".reference-filter-panel")) {
      const header = root.querySelector<HTMLElement>(".references-header")!;
      if (unlinked) header.click();
      root.querySelector<HTMLButtonElement>(".reference-filter-toggle")!.click();
    }
    await vi.waitFor(() => expect(root.querySelectorAll(".ref-filter-chip").length).toBeGreaterThan(0));
  };
  await open();
  return { root, dispose: () => dispose(), reload: async () => { dispose(); dispose = mount(); await open(); } };
}
const chips = (root: HTMLElement) => [...root.querySelectorAll(".ref-filter-chip")].map(c => c.textContent!.trim());
const visible = (root: HTMLElement) => root.querySelector(".references-count")!.textContent;

it.each([false, true])("merges the reporter's 8/3/15 example and counts a block once (overlap=%s)", async overlap => {
  const s = await setup(null, overlap);
  await vi.waitFor(() => expect(chips(s.root)).toEqual(["Germany 26", "Zebra 1"]));
  const chip = s.root.querySelector<HTMLButtonElement>(".ref-filter-chip")!;
  chip.click(); expect(visible(s.root)).toBe("26");
  chip.click(); expect(visible(s.root)).toBe("1");
  expect(readPageProperty("Target", "filters")).toBe('{"Germany" false}');
  await s.reload(); expect(visible(s.root)).toBe("1");
  expect(s.root.querySelector(".f-out")?.textContent).toContain("Germany");
  s.dispose();
});

it("loads alias-named OG filters and writes the canonical name on the next change", async () => {
  const s = await setup('filters:: {"de" true}');
  await vi.waitFor(() => expect(visible(s.root)).toBe("26"));
  expect(chips(s.root)).toEqual(["Germany 26", "Zebra 1"]);
  s.root.querySelector<HTMLButtonElement>(".f-in")!.click();
  expect(readPageProperty("Target", "filters")).toBe('{"Germany" false}');
  s.root.querySelector<HTMLButtonElement>(".reference-filter-clear")!.click();
  expect(readPageProperty("Target", "filters")).toBe("{}");
  s.dispose();
});

it("maps alias-named device filters to the same chip", async () => {
  localStorage.setItem("logseq-claude.refFilters", JSON.stringify({ Target: { DE: "in" } }));
  const s = await setup();
  await vi.waitFor(() => expect(visible(s.root)).toBe("26"));
  expect(chips(s.root)).toEqual(["Germany 26", "Zebra 1"]);
  expect(s.root.querySelector(".f-in")?.textContent).toContain("Germany");
  s.dispose();
});

it("round-trips alias-named filters through the existing Org page-property writer", async () => {
  const s = await setup('#+filters: {"Deutschland" true}', false, false, "org");
  await vi.waitFor(() => expect(visible(s.root)).toBe("26"));
  s.root.querySelector<HTMLButtonElement>(".f-in")!.click();
  expect(readPageProperty("Target", "filters")).toBe('{"Germany" false}');
  await s.reload(); expect(visible(s.root)).toBe("1");
  s.dispose();
});

it("shares alias chips with Unlinked References without writing its state into linked filters", async () => {
  const s = await setup(null, false, true);
  await vi.waitFor(() => expect(chips(s.root)).toEqual(["Germany 26", "Zebra 1"]));
  s.root.querySelector<HTMLButtonElement>(".ref-filter-chip")!.click();
  expect(visible(s.root)).toBe("26");
  expect(readPageProperty("Target", "filters")).toBeNull();
  s.dispose();
});

it("follows live inventory changes and real-page precedence without another graph scan", async () => {
  const s = await setup();
  await vi.waitFor(() => expect(chips(s.root)).toEqual(["Germany 26", "Zebra 1"]));
  vi.mocked(backend().pageInventory).mockResolvedValue({ rev: "2", entries: [
    { name: "Germany", key: "germany", is_journal: false, day: null, target: { kind: "existing", id: "pages/Germany.md", others: [] } },
    { name: "DE", key: "de", is_journal: false, day: null, target: { kind: "existing", id: "pages/DE.md", others: [] } },
    { name: "Deutschland", key: "deutschland", is_journal: false, day: null, target: { kind: "alias", owners: ["pages/Germany.md"] } },
  ] });
  await refreshPageIndex();
  await vi.waitFor(() => expect(chips(s.root)).toEqual(["DE 15", "Germany 11", "Zebra 1"]));
  const reads = vi.mocked(backend().pageInventory).mock.calls.length;
  s.root.querySelector<HTMLButtonElement>(".ref-filter-chip")!.click();
  expect(visible(s.root)).toBe("15");
  expect(vi.mocked(backend().pageInventory).mock.calls.length).toBe(reads);
  s.dispose();
});
