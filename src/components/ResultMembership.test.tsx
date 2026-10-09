import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { EmbedMacro, QueryMacro } from "./Macro";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { BlockReferences } from "./BlockReferences";
import { startEditing, endEdit } from "../editorController";
import { bumpDataRev } from "../graphSession";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetReferenceSectionState } from "../referenceSectionState";
import type { BlockDto } from "../types";

vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: (props: { blocks: BlockDto[] }) => <div data-probe-row>{props.blocks.map((block) => block.raw).join(",")}</div>,
}));
beforeAll(initParser);
afterEach(() => { endEdit("blur"); vi.restoreAllMocks(); resetSharedQueryResultsForTests(); resetReferenceSectionState(); document.body.innerHTML = ""; });
const groups = [{ page: "Source", kind: "page" as const, blocks: [{ id: "task", raw: "TODO my task [[Target]]", children: [], collapsed: false }] }];

it("GH #659 intentional difference from OG: retain the query result until editing ends", async () => {
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacro body="tine-query @block AND task = 'TODO'" />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task"));
    startEditing("task");
    run.mockResolvedValue(blockRunResult([]));
    bumpDataRev();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task");
    endEdit("blur");
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")).toBeNull());
    expect(run).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});

it("GH #660: a disqualified backlink leaves after blur", async () => {
  const read = vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <LinkedReferences name="Target" />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task"));
    startEditing("task"); read.mockResolvedValue([]); bumpDataRev();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task");
    endEdit("blur");
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")).toBeNull());
    expect(read).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});

it.each(["unlinked", "block"] as const)("GH #660 shared lifecycle: %s references retain then remove a disqualified occurrence", async (surface) => {
  const read = surface === "unlinked"
    ? vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups)
    : vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue(groups);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => surface === "unlinked" ? <UnlinkedReferences name="Target" /> : <BlockReferences id="Target" />, root);
  try {
    if (surface === "unlinked") (root.querySelector(".references-header") as HTMLElement).click();
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task"));
    startEditing("task"); read.mockResolvedValue([]); bumpDataRev();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task");
    endEdit("blur");
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")).toBeNull());
    expect(read).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});

it("embed shares retention and refreshes after the editing session ends", async () => {
  const read = vi.spyOn(backend(), "getPage").mockResolvedValue({
    name: "Source", title: "Source", id: "pages/Source.md", kind: "page", format: "md", pre_block: null, blocks: groups[0].blocks,
  });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <EmbedMacro body="embed [[Source]]" />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task"));
    startEditing("task"); read.mockResolvedValue(null); bumpDataRev();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(root.querySelector("[data-probe-row]")?.textContent).toContain("TODO my task");
    endEdit("blur");
    await vi.waitFor(() => expect(root.querySelector("[data-probe-row]")).toBeNull());
    expect(read).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});
