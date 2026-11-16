// F6/F8 (D-20261028-UILP): "Collapse all" lives in the section's header row, and a stale
// count shows an icon with the tooltip, never a bare " ·".
import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { BlockDto, RefGroup } from "../types";
import { ReferenceSection } from "./LinkedReferences";
import { StaleReferenceCount } from "./StaleReferenceCount";
import { resetReferenceSectionState } from "../referenceSectionState";

vi.mock("./LiveRefGroup", () => ({ LiveRefGroup: () => <div class="test-ref-group" /> }));
const block = (id: string): BlockDto => ({ id, raw: "Target", collapsed: false, children: [] });
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
afterEach(() => { document.body.innerHTML = ""; localStorage.clear(); resetReferenceSectionState(); vi.restoreAllMocks(); });

it("puts Collapse all in the header row, left of the copy and filter buttons", async () => {
  const groups: RefGroup[] = ["A", "B"].map((page) => ({ page, kind: "page", blocks: [block(page)] }));
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  const root = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <ReferenceSection name="Target" unlinked={false} />, root);
  try {
    await tick(); await tick();
    const header = root.querySelector<HTMLElement>(".references-header")!;
    const bulk = header.querySelector<HTMLElement>(".reference-bulk-controls")!;
    expect(bulk).not.toBeNull();
    expect(bulk.textContent).toBe("Collapse all");
    expect(root.querySelectorAll(".reference-bulk-controls")).toHaveLength(1);
    const order = [...header.children];
    expect(order.indexOf(bulk)).toBeLessThan(order.indexOf(header.querySelector(".reference-export-toggle")!));
    // pressing it folds the groups without folding the whole section
    bulk.querySelector("button")!.click();
    expect(root.querySelectorAll(".test-ref-group")).toHaveLength(0);
    expect(root.querySelector(".reference-bulk-controls")?.textContent).toBe("Expand all");
  } finally { dispose(); }
});

it("renders the stale mark as an icon with the tooltip and no stray punctuation", () => {
  const root = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <><span class="n">3</span><StaleReferenceCount stale={true} /><StaleReferenceCount stale={false} /></>, root);
  try {
    const marks = root.querySelectorAll<HTMLElement>(".references-count-stale");
    expect(marks).toHaveLength(1);
    expect(marks[0].title).toBe("Count may be out of date");
    expect(marks[0].querySelector("svg")).not.toBeNull();
    expect(marks[0].textContent?.trim()).toBe("");
  } finally { dispose(); }
});
