import { afterEach, describe, expect, it, vi } from "vitest";
import { createResource, ErrorBoundary } from "solid-js";
import { render } from "solid-js/web";
import { applySidebarSession, setRightSidebar } from "../ui";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { RightSidebar } from "./RightSidebar";

const { load } = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock("./Block", async (original) => {
  const actual = await original<typeof import("./Block")>();
  return { ...actual, Block: (props: { id: string }) => {
    const [value] = createResource(() => load(props.id));
    return <div data-test-block={props.id}>{value()}</div>;
  } };
});
vi.mock("./LinkedReferences", () => ({ LinkedReferences: () => <div>Linked references</div> }));
vi.mock("./UnlinkedReferences", () => ({ UnlinkedReferences: () => <div>Unlinked references</div> }));
vi.mock("../debug", () => ({ dbg: () => {}, recordDiagnostic: async () => {} }));

afterEach(() => {
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  vi.clearAllMocks();
});

describe("right sidebar item failure ownership", () => {
  it("a rejected item resource leaves its sibling and navigation usable, then Retry refetches", async () => {
    load.mockImplementation(async (id: string) => {
      if (id === "broken") throw new Error("item resource rejected");
      return "healthy item";
    });
    for (const id of ["broken", "healthy"]) loadSingle({ name: id, kind: "page", title: id, pre_block: null,
      blocks: [{ id, raw: id, collapsed: false, children: [] }] });
    applySidebarSession({ right: true, items: ["broken", "healthy"].map(name => ({ kind: "page", name, pageKind: "page" })) });
    const host = document.createElement("div");
    document.body.appendChild(host);
    // Control containment belongs to the test harness: without the item seam,
    // the whole application subtree is replaced and navigation disappears.
    const dispose = render(() => <ErrorBoundary fallback={<div>Application failed</div>}>
      <button data-navigation>Navigation</button><RightSidebar />
    </ErrorBoundary>, host);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(host.querySelector("[data-navigation]")?.textContent).toBe("Navigation");
    expect(host.querySelector('[data-test-block="healthy"]')?.textContent).toBe("healthy item");
    expect(host.querySelector(".region-failure")?.textContent).toContain("item resource rejected");
    load.mockResolvedValue("recovered item");
    host.querySelector<HTMLButtonElement>(".region-failure-retry")!.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(host.querySelector('[data-test-block="broken"]')?.textContent).toBe("recovered item");
    expect(host.querySelector('[data-test-block="healthy"]')?.textContent).toBe("healthy item");
    dispose();
    host.remove();
  });
});
