import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

vi.mock("../platform", () => ({
  platformKind: async () => "ios" as const,
}));

const { Settings } = await import("./Settings");
const { closeSettings, openSettings } = await import("../ui");

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings iOS plugin boundary (ADR 0052)", () => {
  it("does not expose or initialize the plugin package surface", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);

    openSettings("plugins");
    await tick();
    await tick();

    const tabs = [...root.querySelectorAll(".settings-nav-item")].map((item) => item.textContent);
    expect(tabs).not.toContain("Plugins");
    expect(root.textContent).not.toContain("Install a local package");
    expect(root.textContent).toContain("Theme");
    dispose();
  });
});
