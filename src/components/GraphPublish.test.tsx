import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";

// og H5 (og-G #60): ported from master 350efef1 Settings.publishZero.test.tsx
// (GH #560), adapted to og's Graph-tab publication door (GraphPublish → publishLive).
vi.mock("../owned", async (orig) => ({
  ...(await orig<typeof import("../owned")>()),
  graphOwner: () => () => true,
}));
vi.mock("../sheet/exportSheets", () => ({ exportSheets: async () => [] }));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function exportFromGraphTab(pages: number, everyPage = false, includeComments = true) {
  const { GraphPublish } = await import("./GraphPublish");
  vi.spyOn(backend(), "pickFolder").mockResolvedValue("/mock/out");
  vi.spyOn(backend(), "publishLive").mockResolvedValue({ path: "/mock/out/Tine graph", pages } as never);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <GraphPublish />, root);
  if (everyPage) (root.querySelector('input[type="checkbox"]') as HTMLInputElement).click();
  if (!includeComments) [...root.querySelectorAll("label")].find(label => label.textContent?.includes("Include comments"))!.querySelector("input")!.click();
  const button = [...root.querySelectorAll("button")].find((b) => b.textContent?.includes("Export HTML"));
  expect(button, "the Graph tab must offer the export button").toBeTruthy();
  button!.click();
  for (let i = 0; i < 6; i++) await tick();
  return { root, dispose };
}

afterEach(() => { document.body.innerHTML = ""; vi.restoreAllMocks(); });

describe("og Graph publish (ported GH #560)", () => {
  it("offers comments on by default and sends an explicit exclusion to the shared publisher", async () => {
    const { root, dispose } = await exportFromGraphTab(1, false, false);
    expect(backend().publishLive).toHaveBeenCalledWith("/mock/out", "Tine graph", false, [], false);
    expect([...root.querySelectorAll("label")].find(label => label.textContent?.includes("Include comments"))!.querySelector("input")!.checked).toBe(false);
    dispose();
  });
  it("says why an export produced nothing", async () => {
    const { root, dispose } = await exportFromGraphTab(0);
    const status = root.querySelector('[role="status"]')?.textContent ?? "";
    expect(status).toContain("Exported 0 pages");
    expect(status).toContain("public:: true");
    dispose();
  });
  it("just reports the count when pages were exported", async () => {
    const { root, dispose } = await exportFromGraphTab(3);
    const status = root.querySelector('[role="status"]')?.textContent ?? "";
    expect(status).toContain("Exported 3 pages");
    expect(status).not.toContain("public:: true");
    dispose();
  });
  it("does not blame public:: true when every page was requested", async () => {
    const { root, dispose } = await exportFromGraphTab(0, true);
    const status = root.querySelector('[role="status"]')?.textContent ?? "";
    expect(status).toContain("Exported 0 pages");
    expect(status).not.toContain("public:: true");
    expect(backend().publishLive).toHaveBeenCalledWith("/mock/out", "Tine graph", true, [], true);
    dispose();
  });
});
