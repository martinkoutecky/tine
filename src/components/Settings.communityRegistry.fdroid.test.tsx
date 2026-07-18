import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

// F-Droid build: the recipe sets TINE_COMMUNITY_REGISTRY=0, so vite defines
// __TINE_COMMUNITY_REGISTRY__ = false and COMMUNITY_REGISTRY_ENABLED is false.
// We can't flip a vite `define` per test, so mock the exported flag off and keep
// every other registry export real. Settings reads the flag from this module, so
// the mocked value drives the <Show> gates.
vi.mock("../plugins/registry", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../plugins/registry")>();
  return { ...actual, COMMUNITY_REGISTRY_ENABLED: false };
});

const { Settings } = await import("./Settings");
const { closeSettings, openSettings } = await import("../ui");

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const sectionHeadings = (root: HTMLElement) =>
  [...root.querySelectorAll(".settings-section")].map((el) => el.textContent);

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("community registry — disabled (F-Droid build)", () => {
  it("hides the network catalogue but keeps local plugin sideloading", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("plugins");
    await tick();
    // Local, user-chosen sideloading is unaffected — only the network registry goes.
    expect(root.textContent).toContain("Install a local package");
    expect(sectionHeadings(root)).not.toContain("Community catalogue");
    dispose();
  });

  it("hides theme packages but keeps the built-in theme gallery", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    const headings = sectionHeadings(root);
    expect(headings).toContain("Themes");
    expect(headings).not.toContain("Theme packages");
    dispose();
  });
});
