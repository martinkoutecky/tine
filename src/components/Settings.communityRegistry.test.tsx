import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";

// Default (non-F-Droid) build: __TINE_COMMUNITY_REGISTRY__ is true (vitest
// define mirrors the prod default), so the network-backed community catalogue
// and theme packages are present. The F-Droid-disabled counterpart lives in
// Settings.communityRegistry.fdroid.test.tsx (vi.mock forces the flag off).

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const sectionHeadings = (root: HTMLElement) =>
  [...root.querySelectorAll(".settings-section")].map((el) => el.textContent);

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("community registry — enabled (default build)", () => {
  it("shows the community catalogue on the Plugins tab", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("plugins");
    await tick();
    expect(sectionHeadings(root)).toContain("Community catalogue");
    expect(root.textContent).toContain("Install a local package");
    dispose();
  });

  it("shows theme packages on the Appearance tab", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    expect(sectionHeadings(root)).toContain("Theme packages");
    dispose();
  });
});
