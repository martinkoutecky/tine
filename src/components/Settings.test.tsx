import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings progressive disclosure and search", () => {
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
