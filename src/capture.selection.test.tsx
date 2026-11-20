import { expect, it, vi } from "vitest";
import { backend } from "./backend";
import { CAPTURE_SCRATCH_NAME } from "./captureSeed";
import { insertOutlineAfter, node, pageByName, selectedIds, setRaw, topSelected } from "./document";
import { endEdit } from "./editorController";

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {} }),
}));

it("copies the clicked outline in Capture", async () => {
  const host = document.createElement("div");
  host.id = "capture-root";
  document.body.append(host);
  const writeRich = vi.spyOn(backend(), "writeRich").mockResolvedValue(undefined);
  await import("./capture");
  const first = await vi.waitFor(() => {
    const id = pageByName(CAPTURE_SCRATCH_NAME)?.roots[0];
    expect(id).toBeDefined();
    return id!;
  });
  endEdit("blur");
  setRaw(first, "First capture");
  insertOutlineAfter(first, [
    { raw: "Second capture", children: [{ raw: "Nested capture", children: [] }] },
    { raw: "Third capture", children: [] },
  ]);
  const third = pageByName(CAPTURE_SCRATCH_NAME)!.roots[2];
  const expectedRoots = [...pageByName(CAPTURE_SCRATCH_NAME)!.roots];
  const press = (key: string) => {
    const event = new KeyboardEvent("keydown", { key, ctrlKey: true, bubbles: true, cancelable: true });
    document.body.dispatchEvent(event);
    return event.defaultPrevented;
  };
  const padding = host.querySelector<HTMLElement>(`[data-block-id="${third}"] > .block-main`)!;
  host.querySelector<HTMLElement>(".page-blocks")!.click();
  expect(press("a")).toBe(true);
  expect(topSelected()).toEqual(expectedRoots);
  expect(selectedIds()).toContain(third);
  expect(press("c")).toBe(true);
  expect(writeRich.mock.lastCall?.[0]).toBe("- First capture\n- Second capture\n\t- Nested capture\n- Third capture");
  const html = new DOMParser().parseFromString(writeRich.mock.lastCall![1], "text/html");
  expect(html.querySelector("body > ul > li:nth-child(2) > ul > li")?.textContent).toBe("Nested capture");
  const formats = writeRich.mock.lastCall;
  padding.click();
  expect(press("a")).toBe(true);
  expect(topSelected()).toEqual(expectedRoots);
  expect(press("x")).toBe(true);
  await vi.waitFor(() => expect(expectedRoots.map(id => node(id))).toEqual([undefined, undefined, undefined]));
  expect(writeRich.mock.lastCall).toEqual(formats);
  expect(selectedIds()).toEqual([]);
});
