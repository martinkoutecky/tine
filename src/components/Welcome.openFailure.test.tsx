import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

// The graph chosen at launch would not open: Welcome must say which graph and
// why, and retry it — not look like a first run where nothing was configured.
const { loadGraphPath, writeClipboardText } = vi.hoisted(() => ({
  loadGraphPath: vi.fn<(path: string) => Promise<unknown>>(),
  writeClipboardText: vi.fn<(text: string) => Promise<void>>(async () => {}),
}));
vi.mock("../backend", () => ({ isTauri: () => false }));
vi.mock("./WindowChrome", () => ({ WindowControls: () => null }));
vi.mock("../nativeChrome", () => ({ osDrawsWindowControls: () => true }));
vi.mock("../graph", () => ({ switchGraph: async () => {}, createNewGraph: async () => {}, loadGraphPath }));
vi.mock("../clipboard", () => ({ writeClipboardText }));

import { Welcome } from "./Welcome";
import { setGraphMeta, setStartupOpenFailure, startupOpenFailure } from "../graphSession";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const button = (root: HTMLElement, label: string) =>
  [...root.querySelectorAll("button")].find((b) => b.textContent === label)!;

afterEach(() => {
  setStartupOpenFailure(null);
  setGraphMeta(null);
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

function mount(): { root: HTMLElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(() => <Welcome />, root) };
}

describe("Welcome after a failed launch open", () => {
  it("names the graph and the reason, and stays silent on a genuine first run", () => {
    const first = mount();
    expect(first.root.querySelector(".welcome-recovery")).toBeNull();
    first.dispose();

    setStartupOpenFailure({ path: "/notes/graph", message: "Error: permission denied" });
    const { root, dispose } = mount();
    const card = root.querySelector(".welcome-recovery")!;
    expect(card.getAttribute("role")).toBe("alert");
    expect(card.textContent).toContain("/notes/graph");
    expect(card.textContent).toContain("permission denied");
    dispose();
  });

  it("Try again reloads the failed path; a second failure updates the reason", async () => {
    setStartupOpenFailure({ path: "/notes/graph", message: "first" });
    loadGraphPath.mockRejectedValueOnce(new Error("second"));
    const { root, dispose } = mount();
    button(root, "Try again").click();
    await tick();
    expect(loadGraphPath).toHaveBeenCalledWith("/notes/graph");
    expect(root.querySelector(".welcome-recovery-reason")!.textContent).toContain("second");

    loadGraphPath.mockResolvedValueOnce({ kind: "loaded", root: "/notes/graph" });
    button(root, "Try again").click();
    await tick();
    expect(startupOpenFailure()).toBeNull();
    expect(root.querySelector(".welcome-recovery")).toBeNull();
    dispose();
  });

  it("Copy details copies the path and the reason", async () => {
    setStartupOpenFailure({ path: "/notes/graph", message: "disk gone" });
    const { root, dispose } = mount();
    button(root, "Copy details").click();
    await tick();
    expect(writeClipboardText).toHaveBeenCalledWith("Could not open /notes/graph\ndisk gone");
    dispose();
  });
});
