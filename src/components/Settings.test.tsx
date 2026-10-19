import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { setGraphConfigProblem } from "../graph";
import { closeSettings, openSettings } from "../ui";
import { setActiveLocale } from "../i18n";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  void setActiveLocale("en");
  closeSettings();
  setGraphConfigProblem(null);
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings progressive disclosure and search", () => {
  it.each(["en", "zh-CN"] as const)("shows only matching settings and restores the tab when cleared (%s, GH #706)", async (locale) => {
    await setActiveLocale(locale);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.focus();
    for (const query of [locale === "en" ? "sticky" : "父", "no-such-setting-706"]) {
      search.value = query;
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      expect(root.querySelector(".settings-field")).toBeNull();
      expect(root.querySelector(".settings-search-results")).not.toBeNull();
      expect(document.activeElement).toBe(search);
      if (query === "no-such-setting-706") expect(root.querySelector(".settings-search-empty")).not.toBeNull();
      else {
        expect(root.querySelectorAll(".settings-search-result").length).toBeGreaterThan(0);
        const marks = [...root.querySelectorAll(".settings-search-result mark")];
        expect(marks.length).toBeGreaterThan(0);
        expect(marks.every((mark) => mark.textContent?.toLowerCase() === query.toLowerCase())).toBe(true);
      }
    }
    search.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await tick();
    expect(root.querySelector(".settings-search-results")).toBeNull();
    expect(root.querySelector(".settings-field")).not.toBeNull();
    expect(document.activeElement).toBe(search);
    dispose();
  });

  it("shows the matching description under a description-only result (zh-CN, GH #706)", async () => {
    await setActiveLocale("zh-CN");
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "发";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    const results = [...root.querySelectorAll(".settings-search-result")];
    const graph = results.find((result) => result.querySelector("span")?.firstChild?.textContent === "图谱")!;
    expect(graph).toBeDefined();
    expect(graph.querySelector(".settings-search-description mark")?.textContent).toBe("发");
    const publish = results.find((result) => result.textContent?.startsWith("发布"))!;
    expect(publish.querySelector(".settings-search-description")).toBeNull();
    dispose();
  });

  it.each(["en", "zh-CN"] as const)("opens and scrolls to a translated matching control (%s)", async (locale) => {
    await setActiveLocale(locale);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
    openSettings("editor");
    await tick();
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "sticky";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    root.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await tick();
    expect(search.value).toBe("");
    expect(scroll).toHaveBeenCalled();
    expect((scroll.mock.contexts.at(-1) as HTMLElement).querySelector('[role="switch"]')).not.toBeNull();
    // The setting it landed on is marked for a moment (GH #706).
    expect((scroll.mock.contexts.at(-1) as HTMLElement).closest(".settings-jump-flash")).not.toBeNull();
    expect(document.activeElement).toBe(search);
    scroll.mockRestore();
    dispose();
  });

  it("searches shortcut commands and unbinds a key without hiding its command", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("shortcuts");
    await tick();
    const search = root.querySelector<HTMLInputElement>('.settings-search-input')!;
    search.value = "go to home";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    const row = [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")]
      .find((element) => element.textContent?.includes("Go to home page"))!;
    expect(row).toBeDefined();
    row.querySelector<HTMLButtonElement>('button[title="Remove this keybinding"]')!.click();
    await tick();
    const updated = [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")]
      .find((element) => element.textContent?.includes("Go to home page"))!;
    expect(updated.textContent).toContain("Unbound");
    expect(updated.textContent).toContain("Go to home page");
    dispose();
  });

  it("opens Help & diagnostics with reporting and graph tools, without the parser runner (UI-CENSUS-18)", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("diagnostics");
    await vi.waitFor(() => expect(root.querySelector(".diagnostics-tab h2")?.textContent).toBe("Help & diagnostics"));
    expect(root.textContent).toContain("Create diagnostic report");
    expect(root.querySelector(".improve-run")).toBeNull();
    expect(root.textContent).not.toContain("Help improve Tine's parser");
    for (const label of ["Save report", "Clear recorded events", "Rescan graph", "Verify synchronized graph"]) expect(root.textContent).toContain(label);
    dispose();
  });

  it("remembers Settings maximize across close and reopen", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    root.querySelector<HTMLButtonElement>('button[aria-label="Maximize settings"]')!.click();
    expect(root.querySelector(".settings-maximized")).not.toBeNull();
    closeSettings();
    openSettings("appearance");
    await tick();
    expect(root.querySelector(".settings-maximized")).not.toBeNull();
    dispose();
  });
  it("exposes the accessible three-mode Link autocomplete policy through Settings search", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("editor");
    await tick();

    const search = root.querySelector(".settings-search-input") as HTMLInputElement;
    search.value = "link autocomplete default";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    expect(root.textContent).toContain("Link autocomplete default");
    root.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await tick();
    expect(search.value).toBe("");
    const policy = root.querySelector<HTMLSelectElement>('select[aria-label="Link autocomplete default"]');
    expect(policy?.value).toBe("adaptive");
    expect([...policy!.options].map((option) => option.text)).toEqual([
      "OG adaptive", "Prefer existing", "Prefer exactly what I typed",
    ]);
    policy!.value = "typed";
    policy!.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(policy!.value).toBe("typed");
    dispose();
  });

  it("reveals an Advanced match across tabs and clearing restores the collapsed state", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();

    const search = root.querySelector(".settings-search-input") as HTMLInputElement;
    search.value = "diagram editors";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    const result = root.querySelector(".settings-search-result") as HTMLButtonElement;
    expect(result.textContent).toContain("Files › Advanced");
    result.click();
    await tick();
    const advanced = root.querySelector(".settings-advanced-toggle") as HTMLButtonElement;
    expect(advanced.getAttribute("aria-expanded")).toBe("true");
    expect(root.textContent).toContain("Diagram editors");

    search.value = "";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    expect(advanced.getAttribute("aria-expanded")).toBe("false");
    expect(root.textContent).not.toContain("Edit diagram assets in your own installed app");
    dispose();
  });

  it("persists explicit expansion per tab and supports Escape collapse", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("editor");
    await tick();
    const button = root.querySelector(".settings-advanced-toggle") as HTMLButtonElement;
    button.click();
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(localStorage.getItem("tine.settings.advanced.editor")).toBe("1");
    button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await tick();
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(button);
    expect(localStorage.getItem("tine.settings.advanced.editor")).toBe("0");
    dispose();
  });
});


it("keeps the config-read error visible in Settings until a repaired reopen", async () => {
  setGraphConfigProblem({ kind: "config-read", message: "read failed" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Settings />, host);
  openSettings("appearance");
  await tick();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("This graph is read-only");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Repair the config and reopen");
  setGraphConfigProblem(null);
  await tick();
  expect(host.querySelector('[role="alert"]')).toBeNull();
  dispose();
});
