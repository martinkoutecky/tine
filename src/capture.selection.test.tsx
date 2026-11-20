import { expect, it, vi } from "vitest";
import { backend } from "./backend";
import { CAPTURE_SCRATCH_NAME } from "./captureSeed";
import { insertOutlineAfter, pageByName, selectedIds, setRaw } from "./document";
import { endEdit } from "./editorController";

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {} }),
}));

it("uses the blank selection scope for Shift-click in Capture", async () => {
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
  const [_, second, third] = pageByName(CAPTURE_SCRATCH_NAME)!.roots;
  const press = (key: string) => {
    const event = new KeyboardEvent("keydown", { key, ctrlKey: true, bubbles: true, cancelable: true });
    document.body.dispatchEvent(event);
    return event.defaultPrevented;
  };
  host.querySelector(".page-blocks")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
  expect(press("a")).toBe(true);
  expect(selectedIds()).toContain(third);
  host.querySelector(`[data-block-id="${second}"] > .block-main .block-content-wrapper`)!
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, shiftKey: true }));
  expect(selectedIds()).toHaveLength(3);
  expect(press("c")).toBe(true);
  expect(writeRich.mock.lastCall?.[0]).toBe("- First capture\n- Second capture\n\t- Nested capture");
  const html = new DOMParser().parseFromString(writeRich.mock.lastCall![1], "text/html");
  expect(html.querySelector("body > ul > li:nth-child(2) > ul > li")?.textContent).toBe("Nested capture");
});
