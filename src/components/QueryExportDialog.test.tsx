import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { QueryExportDialog } from "./QueryExportDialog";

vi.mock("../owned", async (orig) => ({ ...await orig<typeof import("../owned")>(), graphOwner: () => () => true }));
vi.mock("../sheet/exportSheets", () => ({ exportSheets: async () => [] }));
vi.mock("../ui", () => ({ closeQueryExport: vi.fn(), openSettings: vi.fn() }));
vi.mock("../toasts", () => ({ pushToast: vi.fn() }));
const tick = () => new Promise((r) => setTimeout(r, 0));
const request = { argument: "(task TODO)", dialect: "macro-query", properties: [], name: "Tasks" } as never;
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.innerHTML = ""; vi.restoreAllMocks(); });
async function dialog(exists = false) {
  vi.spyOn(backend(), "publishQueryPlan").mockResolvedValue({ anchor: "block", rowCount: 1,
    pages: [{ name: "Public", path: "pages/Public.md", journal: false }], folder: "tasks", fingerprint: "reviewed",
    path: "/graph/published-queries/tasks", exists, suggestedFolder: exists ? "tasks-2" : null } as never);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <QueryExportDialog request={() => request} />, root);
  for (let i = 0; i < 4; i++) await tick();
  (root.querySelector('input[type="checkbox"]') as HTMLInputElement).click();
  return root;
}
function exportButton(root: HTMLElement) { return [...root.querySelectorAll("button")].find((b) => b.textContent === "Export")!; }
it("exports a reviewed query directly into its graph and reports recovery and missing assets", async () => {
  const api = vi.spyOn(backend(), "publishQuery").mockResolvedValue({ path: "/graph/published-queries/tasks", pages: 1,
    files: 12, retired: "/graph/logseq/.tine-trash/conflicts/old/previous", warnings: ["missing.png omitted"] } as never);
  const root = await dialog();
  expect(exportButton(root).disabled, "the graph leaf needs no external folder picker").toBe(false);
  exportButton(root).click(); for (let i = 0; i < 4; i++) await tick();
  expect(api).toHaveBeenCalled();
  const { pushToast } = await import("../toasts");
  expect(pushToast).toHaveBeenCalledWith(expect.stringContaining("old/previous"), "success", { sticky: true });
  expect(pushToast).toHaveBeenCalledWith(expect.stringContaining("missing.png"), "warn", { sticky: true });
});
it("requires an explicit Replace or separate folder choice", async () => {
  const api = vi.spyOn(backend(), "publishQuery").mockResolvedValue({ path: "site", pages: 1, files: 12, retired: null, warnings: [] } as never);
  const root = await dialog(true);
  expect(exportButton(root).disabled).toBe(true);
  const radios = root.querySelectorAll<HTMLInputElement>('input[type="radio"]');
  expect(radios.length, "a colliding graph leaf offers Replace and a separate folder").toBe(2);
  radios[1].click(); exportButton(root).click(); for (let i = 0; i < 4; i++) await tick();
  expect(api.mock.calls[0][0]).toMatchObject({ folder: "tasks-2", replace: false });
});
it("offers Settings for the typed asset refusal, without parsing its message", async () => {
  vi.spyOn(backend(), "publishQuery").mockRejectedValue({ kind: "assetBudget", message: "arbitrary translated text" });
  const root = await dialog(); exportButton(root).click(); for (let i = 0; i < 4; i++) await tick();
  const adjust = [...root.querySelectorAll("button")].find((b) => b.textContent === "Adjust limit in Settings…");
  expect(adjust).toBeTruthy(); adjust!.click();
  const { openSettings } = await import("../ui"); expect(openSettings).toHaveBeenCalledWith("graph");
});
