import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { RefGroup } from "../types";
import { UnlinkedReferences } from "./UnlinkedReferences";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("Unlinked References evidence and disclosure (GH #144/#145)", () => {
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
});
