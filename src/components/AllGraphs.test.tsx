import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AllGraphs } from "./AllGraphs";
import { commandDefaults } from "../keybindings";

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("offers All graphs in the shared shortcut settings and command palette", () => {
  expect(commandDefaults().find(c => c.id === "go/all-graphs")?.binding).toBe("g shift+g");
});

it("keeps the page usable when listing fails and retries the existing registry", async () => {
  const list = vi.spyOn(backend(), "listKnownGraphs").mockRejectedValueOnce(new Error("unavailable"))
    .mockResolvedValue([{ name: "Recovered", path: "/recovered" }]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <AllGraphs />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector('[role="alert"]')).not.toBeNull());
    root.querySelector<HTMLButtonElement>(".resource-failure-retry")!.click();
    await vi.waitFor(() => expect(root.querySelector("[data-graph-open]")?.textContent).toBe("Recovered"));
    expect(list).toHaveBeenCalledTimes(2);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  } finally { dispose(); }
});

it("offers keyboard-focusable graph management and refetches after forgetting a graph", async () => {
  vi.spyOn(backend(), "listKnownGraphs").mockResolvedValueOnce([{ name: "Work", path: "/work" }]).mockResolvedValue([]);
  const forget = vi.spyOn(backend(), "forgetKnownGraph").mockResolvedValue(undefined);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <AllGraphs />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("[data-graph-open]")).not.toBeNull());
    const buttons = [...root.querySelectorAll<HTMLButtonElement>(".all-graphs-row button")];
    expect(buttons.some(button => button.textContent === "Open in a new window")).toBe(true);
    const remove = buttons.find(button => button.textContent === "Remove from this list")!;
    expect(remove.tabIndex).toBe(0);
    remove.click();
    await vi.waitFor(() => expect(root.querySelector("[data-graph-open]")).toBeNull());
    expect(forget).toHaveBeenCalledWith("/work");
    expect(root.textContent).toContain("Open graph");
    expect(root.textContent).toContain("New graph");
  } finally { dispose(); }
});

it("focuses graphs, moves with arrows/Home/End and opens with Enter or Shift+Enter", async () => {
  vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
    { name: "Personal", path: "/personal" }, { name: "Work", path: "/work" },
  ]);
  const openKnown = vi.fn(async () => ({ kind: "already_current" as const, root: "/personal" }));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <AllGraphs actions={{ openKnown, openPicked: openKnown, createNew: openKnown }} />, root);
  try {
    await vi.waitFor(() => expect(document.activeElement?.textContent).toBe("Personal"));
    const key = (key: string, shiftKey = false) => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }));
    key("ArrowDown");
    expect(document.activeElement?.textContent).toBe("Work");
    key("Home");
    expect(document.activeElement?.textContent).toBe("Personal");
    key("End"); key("Enter");
    expect(openKnown).toHaveBeenLastCalledWith("/work", false);
    key("ArrowUp"); key("Enter", true);
    expect(openKnown).toHaveBeenLastCalledWith("/personal", true);
  } finally { dispose(); }
});
