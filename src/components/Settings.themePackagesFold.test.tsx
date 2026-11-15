import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { RegistryTheme } from "../plugins/registry";

// GH #882: the community theme packages list pushed the rest of Appearance far down.
// It shows the first few with "Show all", and a Settings search result still reveals a folded theme.
const THEMES: RegistryTheme[] = ["One", "Two", "Three", "Four", "Five", "Six", "Seven"].map((name) => ({
  id: `theme-${name.toLowerCase()}`, name: `Theme ${name}`, description: `${name} palette`, source: "https://example.invalid",
  license: "MIT", aiDevelopment: "none",
  versions: [{ version: "1.0.0", apiVersion: "0.3", modes: ["light"], manifestSha256: "0", manifestUrl: "https://example.invalid/t.json",
    audit: { status: "passed" } as RegistryTheme["versions"][number]["audit"], publishedAt: "2026-11-01" }],
}));

vi.mock("../plugins/registry", async (original) => ({
  ...(await original<typeof import("../plugins/registry")>()),
  COMMUNITY_REGISTRY_ENABLED: true,
  communityThemes: () => THEMES,
}));

const { ThemeSettings } = await import("./ThemeSettings");
const { Settings } = await import("./Settings");
const { closeSettings, openSettings } = await import("../ui");

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const shownThemes = (root: HTMLElement) =>
  [...root.querySelectorAll<HTMLElement>("[data-setting-id^='theme.community.']")].map((el) => el.dataset.settingId);
const toggle = (root: HTMLElement) => root.querySelector<HTMLButtonElement>(".theme-packages-more");

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("theme packages list folds (GH #882)", () => {
  it("shows the first four in the Themes category, then all on Show all, and folds again", async () => {
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <Settings />, root);
    openSettings("themes");
    await tick();
    expect(shownThemes(root)).toEqual(["theme.community.theme-one", "theme.community.theme-two", "theme.community.theme-three", "theme.community.theme-four"]);
    expect(toggle(root)?.textContent?.trim()).toBe("Show all 7 theme packages");
    expect(toggle(root)?.getAttribute("aria-expanded")).toBe("false");
    toggle(root)!.click();
    await tick();
    expect(shownThemes(root)).toHaveLength(7);
    expect(toggle(root)?.textContent?.trim()).toBe("Show fewer theme packages");
    toggle(root)!.click();
    await tick();
    expect(shownThemes(root)).toHaveLength(4);
    dispose();
  });

  it("a search naming a folded theme shows the whole list, so the result can land on it", async () => {
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <ThemeSettings search="Theme Six" />, root);
    await tick();
    expect(shownThemes(root)).toContain("theme.community.theme-six");
    expect(toggle(root)).toBeNull();
    dispose();
  });

  it("a search matching only shown themes keeps the list folded", async () => {
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <ThemeSettings search="Theme One" />, root);
    await tick();
    expect(shownThemes(root)).toHaveLength(4);
    dispose();
  });
});
