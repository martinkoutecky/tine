import { afterEach, expect, it, vi } from "vitest";
import { createResource, ErrorBoundary } from "solid-js";
import { render } from "solid-js/web";
import { PaneTree } from "../App";
import { layoutRoot, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
import { makePdfRoute, type PaneSnapshot } from "../router";

const { load } = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock("./PdfViewer", () => ({ KeyedPdfViewer: (props: { route: () => { filename: string } }) => {
  const [data] = createResource(() => load(props.route().filename));
  return <div data-pdf-test>{data()}</div>;
} }));
vi.mock("../debug", () => ({ dbg: () => {}, recordDiagnostic: async () => {} }));
afterEach(() => { resetPaneLayoutToSingle(); vi.clearAllMocks(); });

it("a rejected PDF resource is contained to its pane and Retry leaves the other pane mounted", async () => {
  load.mockImplementation(async (filename: string) => {
    if (filename === "broken.pdf") throw new Error("PDF resource rejected");
    return "healthy PDF";
  });
  const snapshot = (filename: string): PaneSnapshot => ({ tabs: [{ history: [makePdfRoute(filename, filename)], pos: 0, pinned: false }], activeIndex: 0 });
  restorePaneLayout({ kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
    new Map([["main", snapshot("broken.pdf")], ["pane-2", snapshot("healthy.pdf")]]), "main");
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <ErrorBoundary fallback={<div>Application failed</div>}><PaneTree node={layoutRoot()} path={[]} /></ErrorBoundary>, host);
  await vi.waitFor(() => expect(host.querySelector(".region-failure")?.textContent).toContain("PDF resource rejected"));
  const healthy = host.querySelector('[data-pane-id="pane-2"] [data-pdf-test]');
  expect(healthy?.textContent).toBe("healthy PDF");
  expect(host.querySelectorAll(".pane-tab-bar")).toHaveLength(2);
  load.mockResolvedValue("recovered PDF");
  host.querySelector<HTMLButtonElement>(".region-failure-retry")!.click();
  await vi.waitFor(() => expect(host.querySelector('[data-pane-id="main"] [data-pdf-test]')?.textContent).toBe("recovered PDF"));
  expect(host.querySelector('[data-pane-id="pane-2"] [data-pdf-test]')).toBe(healthy);
  expect(load.mock.calls.filter(([file]) => file === "healthy.pdf")).toHaveLength(1);
  dispose();
  host.remove();
});
