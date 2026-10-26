import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { resetReferenceSectionState } from "../referenceSectionState";

vi.mock("./LiveRefGroup", () => ({ LiveRefGroup: () => <div /> }));

afterEach(() => {
  vi.restoreAllMocks();
  resetReferenceSectionState();
  localStorage.clear();
  document.body.innerHTML = "";
});

it("GH #658: both reference headers and their page groups use the same disclosure", async () => {
  const groups = [{ page: "Source", kind: "page" as const,
    blocks: [{ id: "mention", raw: "Target", collapsed: false, children: [] }] }];
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><LinkedReferences name="Target" /><UnlinkedReferences name="Target" /></>, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".linked-references")).not.toBeNull());
    const headers = [...root.querySelectorAll<HTMLElement>(".references-header")];
    expect(headers).toHaveLength(2);
    const shapes = headers.map((header) => header.querySelector(".ref-collapse svg")?.outerHTML);
    expect(shapes[0]).toBeDefined();
    expect(shapes[1]).toBe(shapes[0]);
    root.querySelector<HTMLElement>(".unlinked-references .references-header")!.click();
    const disclosures = [...root.querySelectorAll(".reference-group-disclosure")];
    expect(disclosures).toHaveLength(2);
    for (const disclosure of disclosures) {
      expect(disclosure.querySelector(".ref-collapse svg")?.outerHTML).toBe(shapes[0]);
      expect(disclosure.getAttribute("aria-expanded")).toBe("true");
      (disclosure as HTMLButtonElement).click();
      expect(disclosure.getAttribute("aria-expanded")).toBe("false");
      expect(disclosure.querySelector(".ref-collapse.collapsed")).not.toBeNull();
    }
  } finally { dispose(); }
});
