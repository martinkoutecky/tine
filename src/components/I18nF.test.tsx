import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { setActiveLocale, t } from "../i18n";
import { backend } from "../backend";
import { PageConflictResolution } from "./ConflictResolution";
import { DiffRowView } from "./DiffRows";
import { QuerySentence } from "./querySheetParts";
import { querySentence, propertyFilter, taskFilter, contentFilter, planningFilter } from "../editor/queryBuilder";
import type { ConflictObject, DiffRow } from "../types";
import type { Filter, Query } from "../editor/queryIr";
import { PdfOutlineTree } from "./PdfOutlineTree";
import { SheetAggregateFooterCell } from "./SheetAggregateFooter";
import { ContextMenu } from "./ContextMenu";
import { closeContextMenu } from "../ui";
import { buildBoardColumns } from "../sheet/boardColumns";

afterEach(async () => { await setActiveLocale("en"); vi.restoreAllMocks(); document.body.replaceChildren(); });
const view = (text: string) => ({ uuid: "", text, child_count: 0 });
const row: DiffRow = { id: "modified", kind: "modified", mine: view("TODO authored mine"), theirs: view("TODO authored theirs"), children: [] };

it("PDF outline actions and an existing empty board column follow locale changes, keeping document labels literal", async () => {
  const [expanded, setExpanded] = createSignal(false);
  const columns = buildBoardColumns([], "tags", [], { workflow: "todo" });
  const host = document.createElement("div");
  const dispose = render(() => <>
    <PdfOutlineTree items={[{ id: "chapter", label: "Authored chapter", destination: null, children: [{ id: "child", label: "Authored child", destination: null, children: [] }] }]}
      expanded={expanded} toggle={() => setExpanded(!expanded())} activate={() => {}} />
    <span data-testid="empty-column">{columns.find(column => column.key === null)?.label}</span>
  </>, host);
  try {
    expect(host.querySelector(".pdf-outline-disclosure")?.getAttribute("aria-label")).toBe("Expand Authored chapter");
    expect(host.querySelector('[data-testid="empty-column"]')?.textContent).toBe("(none)");
    await setActiveLocale("zh-CN");
    expect(host.querySelector(".pdf-outline-disclosure")?.getAttribute("aria-label")).toBe("展开 Authored chapter");
    setExpanded(true);
    expect(host.querySelector(".pdf-outline-disclosure")?.getAttribute("aria-label")).toBe("折叠 Authored chapter");
    expect(host.querySelector('[data-testid="empty-column"]')?.textContent).toBe("（无）");
    await setActiveLocale("en");
    expect(host.querySelector(".pdf-outline-disclosure")?.getAttribute("aria-label")).toBe("Collapse Authored chapter");
  } finally { dispose(); }
});

it.each([false, true])("an open aggregate menu follows the locale (query=%s) and keeps function tokens", async (query) => {
  const choose = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <>
    <SheetAggregateFooterCell ownerId="sheet" columnKey="cost" fn="sum" values={[]}
      query={query ? { fn: "sum", text: "4", set: choose } : undefined} />
    <ContextMenu />
  </>, host);
  const labels = () => [...document.querySelectorAll(".ctx-item")].map(item => item.textContent?.trim());
  try {
    host.querySelector<HTMLButtonElement>(".sheet-aggregate-value")!.click();
    expect(labels()).toContain("None");
    expect(labels()).toContain("✓ Sum");
    await setActiveLocale("zh-CN");
    expect(labels()).toContain("无");
    expect(labels()).toContain("✓ 求和");
    expect(labels()).toContain("平均值");
    if (query) {
      [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(item => item.textContent?.trim() === "平均值")!.click();
      expect(choose).toHaveBeenCalledWith("avg");
    }
  } finally { closeContextMenu(); dispose(); }
});

it("open conflict headings and diff decisions follow the live locale, keeping decisions and authored bodies", async () => {
  const conflict: ConflictObject = { id: "review", source: "sync-copy", kind: "page", page_name: "Authored page", page_path: "pages/Authored page.md", sides: [{ role: "mine", label: "HEAD" }, { role: "theirs", label: "branch", path: "copy.md" }] };
  vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue({ base_rev: "r", conflict_rev: "c", rows: [row], mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false });
  const choose = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <>
    <PageConflictResolution conflict={conflict} />
    <div data-testid="decisions">
      <DiffRowView row={row} depth={0} decisions={{}} setDecision={choose} labels={{ mine: "HEAD", theirs: "branch" }} />
      <DiffRowView row={{ ...row, id: "added", kind: "added" }} depth={0} decisions={{}} setDecision={choose} labels={{ mine: "HEAD", theirs: "branch" }} />
      <DiffRowView row={{ ...row, id: "removed", kind: "removed" }} depth={0} decisions={{}} setDecision={choose} labels={{ mine: "HEAD", theirs: "branch" }} />
    </div>
  </>, host);
  try {
    expect(host.querySelector(".page-conflict-title")?.textContent).toBe("Two versions of this page arrived");
    await setActiveLocale("zh-CN");
    expect(host.querySelector(".page-conflict-title")?.textContent).toBe("收到了此页面的两个版本");
    const buttons = [...host.querySelectorAll<HTMLButtonElement>('[data-testid="decisions"] button.sync-merge-seg')];
    expect(buttons.map(button => button.textContent)).toEqual(expect.arrayContaining(["双方", "保留", "舍弃", "跳过", "引入"]));
    expect(host.textContent).toContain("TODO authored mine");
    expect(host.textContent).toContain("HEAD");
    buttons.find(button => button.textContent === "舍弃")!.click();
    expect(choose).toHaveBeenLastCalledWith("added", "theirs");
    await setActiveLocale("en");
    expect(host.querySelector(".page-conflict-title")?.textContent).toBe("Two versions of this page arrived");
  } finally { dispose(); }
});

it("a resting query reorders whole Chinese clauses and preserves literal chips and query syntax", async () => {
  const authored = "原文 <em>literal</em> \uFFF00\uFFF1";
  const filter: Filter = { kind: "and", items: [taskFilter(["TODO"]), propertyFilter("my-key", authored)] };
  const query: Query = { anchor: "block", filter, source: { kind: "builder" } };
  const host = document.createElement("div");
  const dispose = render(() => <QuerySentence query={query} onOpen={() => {}} />, host);
  const before = JSON.stringify(query);
  try {
    expect(host.textContent).toContain(`Blocks where task: TODO and my-key: ${authored}`);
    await setActiveLocale("zh-CN");
    expect(host.textContent).toContain(`符合任务：TODO且my-key：${authored}的块`);
    expect(host.querySelector("em")).toBeNull();
    expect(JSON.stringify(query)).toBe(before);
    const sentence = (filter: Filter, anchor: "block" | "page" = "block", both = false) => querySentence({ anchor, filter, both }).map(segment => segment.text).join("");
    expect(sentence({ kind: "true" })).toBe("所有块");
    expect(sentence({ kind: "true" }, "page")).toBe("所有页面");
    expect(sentence({ kind: "true" }, "block", true)).toBe("所有页面和块");
    expect(sentence(contentFilter(authored))).toContain(authored);
    expect(sentence(planningFilter("scheduled"))).toContain("计划时间");
    await setActiveLocale("en");
    expect(t("conflicts-prose-block-count", { count: 1200 })).toBe("1200 blocks");
  } finally { dispose(); }
});
