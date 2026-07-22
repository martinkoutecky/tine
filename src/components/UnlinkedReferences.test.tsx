import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { RefGroup } from "../types";
import { resetStore, setDoc } from "../store";
import { editingId, endEdit } from "../editorController";
import { route } from "../router";
import { UnlinkedReferences } from "./UnlinkedReferences";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
});

describe("Unlinked References evidence and disclosure (GH #144/#145)", () => {
  it("routes authored DTO identities and focuses their loaded runtime owner", async () => {
    const runtimeId = "runtime-unlinked";
    const authoredId = "authored-unlinked";
    const path = "pages/Source.md";
    setDoc({
      byId: {
        [runtimeId]: {
          id: runtimeId,
          raw: `Target mention\nid:: ${authoredId}`,
          collapsed: false,
          parent: null,
          page: "Source",
          children: [],
        },
      },
      pages: [{
        name: "Source",
        kind: "page",
        title: "Source",
        preBlock: null,
        roots: [runtimeId],
        path,
        format: "md",
        readOnly: false,
        guide: false,
      }],
      feed: ["Source"],
      loaded: true,
    });
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{
      page: "Source",
      kind: "page",
      path,
      blocks: [{
        id: runtimeId,
        raw: `Target mention\nid:: ${authoredId}`,
        collapsed: false,
        children: [],
        properties: [["Id", authoredId]],
      }],
      evidence: [{
        block_id: runtimeId,
        occurrences: [
          { matched_name: "Target", canonical: "Target", kind: "plain", span: { start: 0, end: 6 }, rule: "plain" },
          { matched_name: "Target", canonical: "Target", kind: "plain", span: { start: 0, end: 6 }, rule: "plain" },
        ],
      }],
    }]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    try {
      root.querySelector<HTMLElement>(".references-header")!.click();
      await tick();
      await tick();
      root.querySelector<HTMLButtonElement>(".reference-occurrence-jump")!.click();
      expect(route()).toMatchObject({ kind: "page", name: "Source", pageKind: "page", path });
      await vi.waitFor(() => expect(editingId()).toBe(runtimeId));
    } finally {
      dispose();
    }
  });

  it("computes eagerly while collapsed and mounts results only after expansion (GH #236)", async () => {
    let resolve!: (groups: RefGroup[]) => void;
    const request = vi.spyOn(backend(), "getUnlinkedRefs").mockImplementation(
      () => new Promise<RefGroup[]>((done) => { resolve = done; })
    );
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    await tick();
    expect(request).toHaveBeenCalledOnce();
    expect(root.querySelector(".unlinked-references")?.textContent).toContain("Loading");

    resolve([{
      page: "Source",
      kind: "page",
      blocks: [{ id: "plain", raw: "Target", collapsed: false, children: [] }],
    }]);
    await tick();
    await tick();
    expect(root.querySelector(".references-count")?.textContent).toBe("1");
    expect(root.querySelector(".reference-excerpt-row")).toBeNull();

    root.querySelector<HTMLElement>(".references-header")!.click();
    expect(root.querySelector(".reference-excerpt-row")).not.toBeNull();
    dispose();
  });

  it("shows bounded highlighted evidence and unmounts collapsed page groups", async () => {
    const text = `${"before ".repeat(50)}Target${" after".repeat(50)}`;
    const start = text.indexOf("Target");
    const groups: RefGroup[] = ["One", "Two"].map((page, index) => ({
      page,
      kind: "page",
      blocks: [{ id: `b${index}`, raw: text, collapsed: false, children: [] }],
      evidence: [{
        block_id: `b${index}`,
        occurrences: [{
          matched_name: "Target",
          canonical: "Target",
          kind: "plain",
          span: { start, end: start + 6 },
          rule: "plain_unicode_boundary",
        }],
      }],
    }));
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    root.querySelector<HTMLElement>(".references-header")!.click();
    await tick();
    await tick();
    expect(root.querySelectorAll(".reference-excerpt-row")).toHaveLength(2);
    expect(root.querySelectorAll("mark")[0]?.textContent).toBe("Target");
    expect(root.querySelector(".reference-excerpt-text")!.textContent!.length).toBeLessThan(text.length);

    const collapseAll = [...root.querySelectorAll<HTMLButtonElement>(".reference-bulk-controls button")]
      .find((button) => button.textContent === "Collapse all")!;
    collapseAll.click();
    expect(root.querySelectorAll(".reference-excerpt-row")).toHaveLength(0);
    expect([...root.querySelectorAll(".reference-group-disclosure")]
      .every((button) => button.getAttribute("aria-expanded") === "false")).toBe(true);

    dispose();
  });

  it("merges duplicate normalized page groups into one header", async () => {
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([
      { page: "Note", kind: "page", blocks: [{ id: "a", raw: "Target", collapsed: false, children: [] }] },
      { page: "NOTE", kind: "page", blocks: [{ id: "b", raw: "Target", collapsed: false, children: [] }] },
    ]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    root.querySelector<HTMLElement>(".references-header")!.click();
    await tick();
    await tick();
    expect(root.querySelectorAll(".reference-group-header")).toHaveLength(1);
    expect(root.querySelectorAll(".reference-excerpt-row")).toHaveLength(2);
    dispose();
  });

  it("shows occurrence truncation totals", async () => {
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{
      page: "Note",
      kind: "page",
      blocks: [{ id: "a", raw: "Target", collapsed: false, children: [] }],
      evidence: [{
        block_id: "a",
        occurrences: [{
          matched_name: "Target",
          canonical: "Target",
          kind: "plain",
          span: { start: 0, end: 6 },
          rule: "plain_og_boundary",
        }],
        total: 70,
        truncated: true,
      }],
    } as unknown as RefGroup]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    root.querySelector<HTMLElement>(".references-header")!.click();
    await tick();
    await tick();
    expect(root.querySelector(".reference-truncation")?.textContent).toContain("Showing 1 of 70");
    expect(root.querySelector(".reference-mention-count")?.textContent).toContain("70 mentions");
    expect(root.querySelector(".reference-occurrence-jump")?.getAttribute("aria-label"))
      .toBe("Jump to mention 1 of 70");
    dispose();
  });

  it("renders a bounded bridge error instead of an empty panel", async () => {
    vi.spyOn(backend(), "getUnlinkedRefs").mockRejectedValue(new Error("result-too-large: 20001 matches"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    root.querySelector<HTMLElement>(".references-header")!.click();
    await tick();
    await tick();
    expect(root.querySelector<HTMLElement>('[role="alert"]')?.textContent).toContain(
      "bounded result limit was exceeded"
    );
    dispose();
  });

  it("does not mislabel an ordinary backend failure as a bounded bridge error", async () => {
    vi.spyOn(backend(), "getUnlinkedRefs").mockRejectedValue(new Error("database unavailable"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnlinkedReferences name="Target" />, root);

    root.querySelector<HTMLElement>(".references-header")!.click();
    await tick();
    await tick();
    const message = root.querySelector<HTMLElement>('[role="alert"]')?.textContent ?? "";
    expect(message).toContain("Couldn’t load references");
    expect(message).not.toContain("bounded result limit");
    dispose();
  });
});
